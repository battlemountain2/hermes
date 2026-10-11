use std::{
    collections::HashMap,
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
};

use super::{ThemeTokens, blend};

pub(super) const MAX_PALETTE_BYTES: usize = 64 * 1024;

pub(super) struct Palette {
    pub path: PathBuf,
    pub last_valid: Option<ThemeTokens>,
}

impl Palette {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            last_valid: None,
        }
    }

    pub fn refresh(&mut self) -> bool {
        let Some(tokens) = read_palette(&self.path) else {
            return false;
        };
        if self.last_valid.as_ref() == Some(&tokens) {
            return false;
        }
        self.last_valid = Some(tokens);
        true
    }
}

pub(super) fn palette_path(config: &Path) -> PathBuf {
    config.join("gtk-4.0/noctalia.css")
}

pub(super) fn watch_directory(path: &Path) -> Option<PathBuf> {
    path.ancestors()
        .skip(1)
        .find(|parent| parent.is_dir())
        .map(Path::to_path_buf)
}

pub(super) fn relevant_event(path: &Path, changed: &Path) -> bool {
    path.starts_with(changed)
}

fn read_palette(path: &Path) -> Option<ThemeTokens> {
    let metadata = fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_PALETTE_BYTES as u64 {
        return None;
    }
    let file = File::open(path).ok()?;
    let before = file.metadata().ok()?;
    if !before.is_file() {
        return None;
    }
    let mut source = String::new();
    (&file)
        .take((MAX_PALETTE_BYTES + 1) as u64)
        .read_to_string(&mut source)
        .ok()?;
    let after = file.metadata().ok()?;
    if before.len() != after.len() || before.modified().ok()? != after.modified().ok()? {
        return None;
    }
    tokens_from_css(&source)
}

pub(super) fn tokens_from_css(source: &str) -> Option<ThemeTokens> {
    if source.len() > MAX_PALETTE_BYTES {
        return None;
    }
    let mut uncommented = String::new();
    let mut remaining = source;
    while let Some(start) = remaining.find("/*") {
        uncommented.push_str(&remaining[..start]);
        let end = remaining[start + 2..].find("*/")? + start + 4;
        uncommented.push(' ');
        remaining = &remaining[end..];
    }
    uncommented.push_str(remaining);
    let mut definitions = HashMap::new();
    let mut remaining = uncommented.as_str();
    while let Some(start) = remaining.find("@define-color") {
        remaining = &remaining[start + "@define-color".len()..];
        if !remaining.starts_with(char::is_whitespace) {
            return None;
        }
        let end = remaining.find(';')?;
        let mut fields = remaining[..end].split_whitespace();
        let name = fields.next()?;
        let value = fields.next()?;
        if fields.next().is_some()
            || definitions.insert(name, value).is_some()
            || definitions.len() > 128
        {
            return None;
        }
        remaining = &remaining[end + 1..];
    }
    let resolve = |name: &str| {
        let mut value = *definitions.get(name)?;
        for _ in 0..definitions.len() {
            if let Some(alias) = value.strip_prefix('@') {
                value = *definitions.get(alias)?;
            } else {
                return normalize_hex(value);
            }
        }
        None
    };
    for name in definitions.keys() {
        resolve(name)?;
    }
    let background = resolve("window_bg_color")?;
    let surface = resolve("card_bg_color")?;
    let text = resolve("window_fg_color")?;
    let accent = resolve("accent_bg_color")?;
    let danger = resolve("destructive_bg_color")?;
    Some(ThemeTokens {
        name: "Noctalia".to_owned(),
        muted: blend(&surface, &text, 0.08),
        highlight: blend(&surface, &accent, 0.18),
        border: blend(&surface, &text, 0.24),
        dim_text: blend(&surface, &text, 0.65),
        background,
        surface,
        text,
        accent,
        danger,
    })
}

fn normalize_hex(value: &str) -> Option<String> {
    let hex = value.strip_prefix('#')?;
    if !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    match hex.len() {
        3 => Some(
            format!(
                "#{}",
                hex.chars()
                    .flat_map(|character| [character, character])
                    .collect::<String>()
            )
            .to_ascii_lowercase(),
        ),
        6 => Some(value.to_ascii_lowercase()),
        _ => None,
    }
}

#[cfg(test)]
pub(super) mod tests;
