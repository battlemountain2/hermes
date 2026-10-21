use std::{
    collections::{HashMap, HashSet},
    fs::File,
    io::Read,
    path::Path,
};

use quick_xml::{
    Reader,
    encoding::Decoder,
    events::{BytesStart, Event},
};

const INPUT_LIMIT: u64 = 50 * 1024 * 1024;
const MEMBER_LIMIT: u64 = 16 * 1024 * 1024;
const EXPANDED_LIMIT: u64 = 64 * 1024 * 1024;
const ENTRY_LIMIT: usize = 4096;
const TEXT_LIMIT: usize = 4 * 1024 * 1024;
const SLIDE_LIMIT: usize = 256;
const WORKSHEET_CELL_LIMIT: u64 = 1_000_000;

pub(crate) struct Package {
    archive: zip::ZipArchive<File>,
    remaining: u64,
}

impl Package {
    pub(crate) fn open(file: File) -> Result<Self, String> {
        if file.metadata().map_err(|error| error.to_string())?.len() > INPUT_LIMIT {
            return Err("Office package exceeds the input byte limit".into());
        }
        let archive = zip::ZipArchive::new(file).map_err(|error| error.to_string())?;
        if archive.len() > ENTRY_LIMIT {
            return Err("Office package contains too many members".into());
        }
        let mut package = Self {
            archive,
            remaining: EXPANDED_LIMIT,
        };
        let mut expanded = 0u64;
        let mut names = HashSet::new();
        let mut relationships = Vec::new();
        for index in 0..package.archive.len() {
            let member = package
                .archive
                .by_index(index)
                .map_err(|error| error.to_string())?;
            let name = member.name();
            if name.starts_with('/')
                || name.contains(['\\', ':'])
                || name.split('/').any(|part| part == "..")
                || !names.insert(name.to_owned())
                || member.encrypted()
                || member.is_symlink()
            {
                return Err("Unsafe or ambiguous Office package member".into());
            }
            check_expansion(member.size(), member.compressed_size(), &mut expanded)?;
            if name.ends_with(".rels") {
                relationships.push(name.to_owned());
            }
        }
        for name in relationships {
            let xml = package.read(&name)?;
            visit_xml(&xml, |event, decoder| {
                if let Event::Start(tag) | Event::Empty(tag) = event {
                    if tag.local_name().as_ref() == b"Relationship" {
                        let mode = attribute(&tag, b"TargetMode", decoder)?.unwrap_or_default();
                        let target = attribute(&tag, b"Target", decoder)?.unwrap_or_default();
                        if mode.eq_ignore_ascii_case("external") || external_target(&target) {
                            let kind = attribute(&tag, b"Type", decoder)?.unwrap_or_default();
                            if kind.ends_with("/hyperlink") || kind.ends_with("/image") {
                                return Ok(());
                            }
                            return Err("External Office data references are not supported".into());
                        }
                        let base = name
                            .rsplit_once("/_rels/")
                            .map(|(base, _)| base)
                            .unwrap_or("");
                        package_target(base, &target)?;
                    }
                }
                Ok(())
            })?;
        }
        Ok(package)
    }

    pub(crate) fn contains(&self, name: &str) -> bool {
        self.archive.index_for_name(name).is_some()
    }

    pub(crate) fn read(&mut self, name: &str) -> Result<Vec<u8>, String> {
        let member = self
            .archive
            .by_name(name)
            .map_err(|error| error.to_string())?;
        let limit = MEMBER_LIMIT.min(self.remaining);
        if member.size() > limit {
            return Err("Office XML exceeds the read budget".into());
        }
        let mut bytes = Vec::new();
        member
            .take(limit + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| error.to_string())?;
        if bytes.len() as u64 > limit {
            return Err("Office XML exceeds the read budget".into());
        }
        self.remaining -= bytes.len() as u64;
        Ok(bytes)
    }

    pub(crate) fn validate_spreadsheet_xml(&mut self) -> Result<(), String> {
        let worksheets = self.relationship_targets("/worksheet")?;
        let names: Vec<_> = self
            .archive
            .file_names()
            .filter(|name| name.ends_with(".xml") || worksheets.contains(*name))
            .map(str::to_owned)
            .collect();
        for name in names {
            let xml = self.read(&name)?;
            let mut worksheet = worksheets.contains(&name);
            let mut minimum = (u64::MAX, u64::MAX);
            let mut maximum = (0u64, 0u64);
            let mut row = 1u64;
            let mut column = 1u64;
            visit_xml(&xml, |event, decoder| {
                if let Event::End(tag) = &event {
                    if worksheet && tag.local_name().as_ref() == b"row" {
                        row = row.saturating_add(1);
                        column = 1;
                    }
                }
                if let Event::Start(tag) | Event::Empty(tag) = event {
                    reject_external_attributes(&tag, decoder)?;
                    worksheet |= tag.local_name().as_ref() == b"worksheet";
                    if worksheet && tag.local_name().as_ref() == b"row" {
                        if let Some(reference) = attribute(&tag, b"r", decoder)? {
                            row = reference.parse().map_err(|_| "Invalid worksheet row")?;
                            if row == 0 || row > 1_048_576 {
                                return Err("Worksheet row is outside Excel limits".into());
                            }
                        }
                    }
                    if tag.local_name().as_ref() == b"sst"
                        && let Some(count) = attribute(&tag, b"uniqueCount", decoder)?
                        && count
                            .parse::<u64>()
                            .map_err(|_| "Invalid shared string count")?
                            > WORKSHEET_CELL_LIMIT
                    {
                        return Err("Workbook shared strings exceed the allocation budget".into());
                    }
                    if worksheet && tag.local_name().as_ref() == b"c" {
                        let position = attribute(&tag, b"r", decoder)?
                            .map(|reference| cell_position(&reference))
                            .transpose()?
                            .unwrap_or((row, column));
                        if position.0 == 0
                            || position.0 > 1_048_576
                            || position.1 == 0
                            || position.1 > 16_384
                        {
                            return Err("Worksheet cell reference is outside Excel limits".into());
                        }
                        column = position.1.saturating_add(1);
                        minimum = (minimum.0.min(position.0), minimum.1.min(position.1));
                        maximum = (maximum.0.max(position.0), maximum.1.max(position.1));
                        if (maximum.0 - minimum.0 + 1).saturating_mul(maximum.1 - minimum.1 + 1)
                            > WORKSHEET_CELL_LIMIT
                        {
                            return Err("Sparse worksheet exceeds the allocation budget".into());
                        }
                    }
                }
                Ok(())
            })?;
        }
        Ok(())
    }

    fn relationship_targets(&mut self, kind: &str) -> Result<HashSet<String>, String> {
        let names: Vec<_> = self
            .archive
            .file_names()
            .filter(|name| name.ends_with(".rels"))
            .map(str::to_owned)
            .collect();
        let mut targets = HashSet::new();
        for name in names {
            let xml = self.read(&name)?;
            let base = name.rsplit_once("/_rels/").map_or("", |(base, _)| base);
            visit_xml(&xml, |event, decoder| {
                if let Event::Start(tag) | Event::Empty(tag) = event {
                    if tag.local_name().as_ref() == b"Relationship"
                        && attribute(&tag, b"Type", decoder)?
                            .is_some_and(|value| value.ends_with(kind))
                    {
                        let target = attribute(&tag, b"Target", decoder)?
                            .ok_or("Missing Office relationship target")?;
                        targets.insert(package_target(base, &target)?);
                    }
                }
                Ok(())
            })?;
        }
        Ok(targets)
    }

    pub(crate) fn workbook_part(&mut self) -> Result<Option<String>, String> {
        let mut target = None;
        if self.contains("_rels/.rels") {
            let xml = self.read("_rels/.rels")?;
            visit_xml(&xml, |event, decoder| {
                if let Event::Start(tag) | Event::Empty(tag) = event {
                    if tag.local_name().as_ref() == b"Relationship"
                        && attribute(&tag, b"Type", decoder)?
                            .is_some_and(|value| value.ends_with("/officeDocument"))
                    {
                        if target.is_some() {
                            return Err("Ambiguous Office document relationship".into());
                        }
                        target = Some(package_target(
                            "",
                            &attribute(&tag, b"Target", decoder)?
                                .ok_or("Missing Office document target")?,
                        )?);
                    }
                }
                Ok(())
            })?;
        }
        let target = target.unwrap_or_else(|| "xl/workbook.xml".to_owned());
        if !self.contains(&target) {
            return Ok(None);
        }
        let xml = self.read(&target)?;
        let mut workbook = false;
        let mut first = true;
        visit_xml(&xml, |event, _| {
            if let Event::Start(tag) | Event::Empty(tag) = event {
                if first {
                    workbook = tag.local_name().as_ref() == b"workbook";
                    first = false;
                }
            }
            Ok(())
        })?;
        Ok(workbook.then_some(target))
    }
}

fn check_expansion(size: u64, compressed: u64, expanded: &mut u64) -> Result<(), String> {
    *expanded = expanded.saturating_add(size);
    if size > MEMBER_LIMIT
        || *expanded > EXPANDED_LIMIT
        || size > compressed.max(1).saturating_mul(200)
    {
        return Err("Office package exceeds the expansion budget".into());
    }
    Ok(())
}

fn external_target(target: &str) -> bool {
    target.contains(':') || target.starts_with("//") || target.contains('\\')
}

fn cell_position(reference: &str) -> Result<(u64, u64), String> {
    let split = reference
        .find(|character: char| !character.is_ascii_alphabetic())
        .ok_or("Invalid worksheet cell reference")?;
    let (letters, digits) = reference.split_at(split);
    let mut column = 0u64;
    for letter in letters.bytes() {
        column = column
            .saturating_mul(26)
            .saturating_add(u64::from(letter.to_ascii_uppercase() - b'A' + 1));
    }
    let row = digits.parse::<u64>().map_err(|_| "Invalid worksheet row")?;
    if column == 0 || column > 16_384 || row == 0 || row > 1_048_576 {
        return Err("Worksheet cell reference is outside Excel limits".into());
    }
    Ok((row, column))
}

fn attribute(
    tag: &BytesStart<'_>,
    name: &[u8],
    decoder: Decoder,
) -> Result<Option<String>, String> {
    for attribute in tag.attributes() {
        let attribute = attribute.map_err(|error| error.to_string())?;
        if attribute.key.as_ref() == name {
            return attribute
                .decoded_and_normalized_value(quick_xml::XmlVersion::Implicit1_0, decoder)
                .map(|value| Some(value.into_owned()))
                .map_err(|error| error.to_string());
        }
    }
    Ok(None)
}

fn reject_external_attributes(tag: &BytesStart<'_>, decoder: Decoder) -> Result<(), String> {
    if tag.local_name().as_ref() == b"a" {
        return Ok(());
    }
    for attribute in tag.attributes() {
        let attribute = attribute.map_err(|error| error.to_string())?;
        if attribute.key.local_name().as_ref() == b"href" {
            let target = attribute
                .decoded_and_normalized_value(quick_xml::XmlVersion::Implicit1_0, decoder)
                .map_err(|error| error.to_string())?;
            if external_target(&target) || target.starts_with('/') || target.starts_with("../") {
                return Err("External Office references are not supported".into());
            }
            package_target("", &target)?;
        }
    }
    Ok(())
}

fn visit_xml(
    xml: &[u8],
    mut visitor: impl FnMut(Event<'_>, Decoder) -> Result<(), String>,
) -> Result<(), String> {
    let mut reader = Reader::from_reader(xml);
    reader.config_mut().expand_empty_elements = true;
    let mut depth = 0usize;
    let mut events = 0usize;
    loop {
        let event = reader.read_event().map_err(|error| error.to_string())?;
        events += 1;
        if events > 1_000_000 {
            return Err("Office XML exceeds the event budget".into());
        }
        match &event {
            Event::DocType(_) => return Err("Office XML document types are not supported".into()),
            Event::Start(_) => {
                depth += 1;
                if depth > 256 {
                    return Err("Office XML is nested too deeply".into());
                }
            }
            Event::End(_) => depth = depth.checked_sub(1).ok_or("Invalid Office XML nesting")?,
            Event::Eof => {
                return if depth == 0 {
                    Ok(())
                } else {
                    Err("Incomplete Office XML".into())
                };
            }
            _ => {}
        }
        visitor(event, reader.decoder())?;
    }
}

struct Text {
    value: String,
    limit: usize,
    full: bool,
}

impl Text {
    fn new(limit: usize) -> Self {
        Self {
            value: String::new(),
            limit: limit.min(TEXT_LIMIT),
            full: false,
        }
    }

    fn push(&mut self, text: &str) {
        if self.full {
            return;
        }
        let mut end = text.len().min(self.limit - self.value.len());
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        self.value.push_str(&text[..end]);
        self.full = end < text.len() || self.value.len() == self.limit;
    }

    fn boundary(&mut self, trim: char, separator: &str) {
        if self.full {
            return;
        }
        while self.value.ends_with(trim) {
            self.value.pop();
        }
        self.push(separator);
    }
}

fn append_xml(xml: &[u8], odf: bool, text: &mut Text) -> Result<(), String> {
    let mut capture = 0usize;
    visit_xml(xml, |event, decoder| {
        if odf && let Event::Start(tag) | Event::Empty(tag) = &event {
            reject_external_attributes(tag, decoder)?;
        }
        match event {
            Event::Start(tag) if matches!(tag.local_name().as_ref(), b"p" | b"h") && odf => {
                capture += 1
            }
            Event::Start(tag) if tag.local_name().as_ref() == b"t" && !odf => capture += 1,
            Event::Start(tag) | Event::Empty(tag) => match tag.local_name().as_ref() {
                b"tab" => text.push("\t"),
                b"br" | b"cr" | b"line-break" => text.push("\n"),
                b"s" if odf && capture > 0 => {
                    let count = tag
                        .attributes()
                        .find_map(|attribute| {
                            attribute
                                .ok()
                                .filter(|attribute| attribute.key.local_name().as_ref() == b"c")
                        })
                        .map(|attribute| {
                            attribute
                                .decoded_and_normalized_value(
                                    quick_xml::XmlVersion::Implicit1_0,
                                    decoder,
                                )
                                .map_err(|error| error.to_string())
                        })
                        .transpose()?
                        .and_then(|value| value.parse::<usize>().ok())
                        .unwrap_or(1);
                    text.push(
                        &" ".repeat(
                            count
                                .min(TEXT_LIMIT)
                                .min(text.limit.saturating_sub(text.value.len())),
                        ),
                    );
                }
                _ => {}
            },
            Event::End(tag) => match tag.local_name().as_ref() {
                b"t" if !odf => capture = capture.saturating_sub(1),
                b"p" | b"h" => {
                    if odf {
                        capture = capture.saturating_sub(1);
                    }
                    text.push("\n");
                }
                b"tc" | b"table-cell" => text.boundary('\n', "\t"),
                b"tr" | b"table-row" => text.boundary('\t', "\n"),
                _ => {}
            },
            Event::Text(value) if capture > 0 => {
                text.push(&value.decode().map_err(|error| error.to_string())?)
            }
            Event::CData(value) if capture > 0 => {
                text.push(&value.decode().map_err(|error| error.to_string())?)
            }
            Event::GeneralRef(reference) if capture > 0 => {
                if let Some(character) = reference
                    .resolve_char_ref()
                    .map_err(|error| error.to_string())?
                {
                    text.push(&character.to_string());
                } else {
                    let name = reference.decode().map_err(|error| error.to_string())?;
                    let escaped = format!("&{name};");
                    text.push(
                        &quick_xml::escape::unescape(&escaped)
                            .map_err(|error| error.to_string())?,
                    );
                }
            }
            _ => {}
        }
        Ok(())
    })
}

fn package_target<'a>(base: &'a str, target: &'a str) -> Result<String, String> {
    let mut parts: Vec<_> = if target.starts_with('/') {
        Vec::new()
    } else {
        base.split('/').filter(|part| !part.is_empty()).collect()
    };
    for part in target.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop().ok_or("Reference path escapes Office package")?;
            }
            _ => parts.push(part),
        }
    }
    Ok(parts.join("/"))
}

fn slide_target(target: &str) -> Result<String, String> {
    if target.len() > 1024 || external_target(target) || target.contains(['%', '?', '#']) {
        return Err("Unsafe slide relationship".into());
    }
    let path = package_target("ppt", target)?;
    if !path.starts_with("ppt/slides/") || !path.ends_with(".xml") {
        return Err("Unsupported slide relationship".into());
    }
    Ok(path)
}

fn append_slides(package: &mut Package, text: &mut Text) -> Result<(), String> {
    let relationships = package.read("ppt/_rels/presentation.xml.rels")?;
    let mut targets = HashMap::new();
    visit_xml(&relationships, |event, decoder| {
        if let Event::Start(tag) | Event::Empty(tag) = event {
            if tag.local_name().as_ref() == b"Relationship"
                && attribute(&tag, b"Type", decoder)?.is_some_and(|kind| kind.ends_with("/slide"))
            {
                let id = attribute(&tag, b"Id", decoder)?.ok_or("Missing slide relationship ID")?;
                let target = attribute(&tag, b"Target", decoder)?.ok_or("Missing slide target")?;
                if attribute(&tag, b"TargetMode", decoder)?
                    .is_some_and(|mode| mode.eq_ignore_ascii_case("external"))
                {
                    return Err("External slide references are not supported".into());
                }
                if targets.insert(id, slide_target(&target)?).is_some() {
                    return Err("Duplicate slide relationship ID".into());
                }
            }
        }
        Ok(())
    })?;
    let presentation = package.read("ppt/presentation.xml")?;
    let mut slides = Vec::new();
    let mut slide_count = 0usize;
    visit_xml(&presentation, |event, decoder| {
        if let Event::Start(tag) | Event::Empty(tag) = event {
            if tag.local_name().as_ref() == b"sldId" {
                let id = tag
                    .attributes()
                    .find_map(|attribute| {
                        attribute
                            .ok()
                            .filter(|attribute| attribute.key.as_ref().ends_with(b":id"))
                    })
                    .ok_or("Missing presentation slide ID")?
                    .decoded_and_normalized_value(quick_xml::XmlVersion::Implicit1_0, decoder)
                    .map_err(|error| error.to_string())?
                    .into_owned();
                let target = targets.get(&id).ok_or("Missing slide relationship")?;
                slide_count += 1;
                if slides.len() < SLIDE_LIMIT {
                    slides.push(target.clone());
                }
            }
        }
        Ok(())
    })?;
    if slides.is_empty() {
        return Err("Presentation contains no slides".into());
    }
    for (index, slide) in slides.iter().take(SLIDE_LIMIT).enumerate() {
        if text.full {
            break;
        }
        if index > 0 {
            text.push("\n");
        }
        text.push(&format!("Slide {}\n", index + 1));
        append_xml(&package.read(slide)?, false, text)?;
    }
    if slide_count > SLIDE_LIMIT {
        text.push("\n…additional slides omitted");
    }
    Ok(())
}

pub(crate) fn extract_text(path: &Path, byte_limit: usize) -> Result<Vec<u8>, String> {
    let mut package = Package::open(File::open(path).map_err(|error| error.to_string())?)?;
    let mut text = Text::new(byte_limit);
    if package.contains("word/document.xml") {
        append_xml(&package.read("word/document.xml")?, false, &mut text)?;
    } else if package.contains("ppt/presentation.xml") {
        append_slides(&mut package, &mut text)?;
    } else if package.contains("content.xml") {
        append_xml(&package.read("content.xml")?, true, &mut text)?;
    } else {
        return Err("This Office package structure is not supported".into());
    }
    Ok(text.value.trim().as_bytes().to_vec())
}

#[cfg(test)]
mod tests;
