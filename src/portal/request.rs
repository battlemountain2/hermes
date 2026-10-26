use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    os::unix::ffi::OsStringExt,
    path::PathBuf,
};

use gtk::glib;

pub(crate) type Options = BTreeMap<String, glib::Variant>;
pub(crate) type Filter = (String, Vec<(u32, String)>);
pub(crate) type Choice = (String, String, Vec<(String, String)>, String);

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum Kind {
    Open { directory: bool, multiple: bool },
    Save,
    SaveFiles(Vec<OsString>),
}

#[derive(Clone, Debug)]
pub(crate) struct Request {
    pub kind: Kind,
    pub title: String,
    pub accept_label: String,
    pub modal: bool,
    pub parent: String,
    pub activation_token: Option<String>,
    pub folder: Option<PathBuf>,
    pub file: Option<PathBuf>,
    pub name: Option<String>,
    pub filters: Vec<Filter>,
    pub current_filter: Option<Filter>,
    pub choices: Vec<Choice>,
}

fn bounded(value: &str, limit: usize) -> bool {
    value.len() <= limit && !value.chars().any(char::is_control)
}

fn option<Value: glib::variant::FromVariant>(
    options: &Options,
    name: &str,
) -> Result<Option<Value>, String> {
    options
        .get(name)
        .map(|value| {
            value
                .get::<Value>()
                .ok_or_else(|| format!("Invalid {name} option type"))
        })
        .transpose()
}

fn string(options: &Options, name: &str) -> Result<Option<String>, String> {
    let value = option::<String>(options, name)?;
    if value.as_ref().is_some_and(|value| !bounded(value, 4096)) {
        return Err(format!("Invalid or oversized {name}"));
    }
    Ok(value)
}

pub(crate) fn filename(bytes: &[u8]) -> bool {
    !bytes.is_empty()
        && bytes.len() <= 255
        && bytes != b"."
        && bytes != b".."
        && !bytes.contains(&b'/')
        && !bytes.contains(&0)
}

fn terminated(bytes: Vec<u8>) -> Result<OsString, String> {
    let Some((&0, bytes)) = bytes.split_last() else {
        return Err("Expected a null-terminated native path".into());
    };
    if bytes.is_empty() || bytes.len() > 4096 || bytes.contains(&0) {
        return Err("Invalid or oversized native path".into());
    }
    Ok(OsString::from_vec(bytes.to_vec()))
}

fn path(options: &Options, name: &str) -> Result<Option<PathBuf>, String> {
    let path = option::<Vec<u8>>(options, name)?
        .map(terminated)
        .transpose()?
        .map(PathBuf::from);
    if path.as_ref().is_some_and(|path| !path.is_absolute()) {
        return Err(format!("{name} must be an absolute native path"));
    }
    Ok(path)
}

fn validate_filter(filter: &Filter, remaining: &mut usize) -> Result<(), String> {
    if !bounded(&filter.0, 256) || filter.1.is_empty() || filter.1.len() > *remaining {
        return Err("Too many or invalid file filters".into());
    }
    *remaining -= filter.1.len();
    for (kind, rule) in &filter.1 {
        if !bounded(rule, 256)
            || rule.is_empty()
            || (*kind == 0 && rule.bytes().filter(|value| *value == b'*').count() > 2)
            || (*kind == 1 && !rule.contains('/'))
            || *kind > 1
        {
            return Err("Unsupported or overly complex file filter".into());
        }
    }
    Ok(())
}

impl Request {
    pub(crate) fn parse(
        method: &str,
        title: String,
        parent: String,
        options: Options,
    ) -> Result<Self, String> {
        if !bounded(&title, 4096) || !bounded(&parent, 4096) {
            return Err("Invalid or oversized chooser title/parent".into());
        }
        if !parent.is_empty() && !parent.starts_with("wayland:") && !parent.starts_with("x11:") {
            return Err("Unsupported parent-window identifier".into());
        }
        if parent == "wayland:" || parent == "x11:" {
            return Err("Empty parent-window identifier".into());
        }
        let kind = match method {
            "OpenFile" => Kind::Open {
                directory: option::<bool>(&options, "directory")?.unwrap_or(false),
                multiple: option::<bool>(&options, "multiple")?.unwrap_or(false),
            },
            "SaveFile" => Kind::Save,
            "SaveFiles" => {
                let names =
                    option::<Vec<Vec<u8>>>(&options, "files")?.ok_or("Missing files to save")?;
                if names.is_empty() || names.len() > 64 {
                    return Err("Choose between 1 and 64 output files".into());
                }
                let names = names
                    .into_iter()
                    .map(terminated)
                    .collect::<Result<Vec<_>, _>>()?;
                if names.iter().any(|name| !filename(name.as_encoded_bytes())) {
                    return Err("Output names must be plain filenames, not paths".into());
                }
                Kind::SaveFiles(names)
            }
            _ => return Err("Unknown file-chooser method".into()),
        };
        let filters = option::<Vec<Filter>>(&options, "filters")?.unwrap_or_default();
        let current_filter = option::<Filter>(&options, "current_filter")?;
        if filters.len() > 32 {
            return Err("Too many file filters".into());
        }
        let mut remaining = 256;
        for filter in filters.iter().chain(current_filter.iter()) {
            validate_filter(filter, &mut remaining)?;
        }
        let choices = option::<Vec<Choice>>(&options, "choices")?.unwrap_or_default();
        if choices.len() > 16 {
            return Err("Too many chooser options".into());
        }
        let mut identifiers = BTreeSet::new();
        let mut total = 0;
        for (id, label, values, selected) in &choices {
            total += values.len();
            let mut value_ids = BTreeSet::new();
            if id.is_empty()
                || !bounded(id, 128)
                || !bounded(label, 512)
                || !identifiers.insert(id)
                || values.len() > 32
                || total > 128
                || values.iter().any(|(id, label)| {
                    id.is_empty()
                        || !bounded(id, 128)
                        || !bounded(label, 512)
                        || !value_ids.insert(id)
                })
                || (values.is_empty() && !matches!(selected.as_str(), "" | "true" | "false"))
                || (!values.is_empty()
                    && !selected.is_empty()
                    && !values.iter().any(|(id, _)| id == selected))
            {
                return Err("Invalid, duplicate or oversized chooser options".into());
            }
        }
        let name = string(&options, "current_name")?;
        if name.as_ref().is_some_and(|name| !filename(name.as_bytes())) {
            return Err("The suggested name must be a plain filename".into());
        }
        let default_label = match kind {
            Kind::Open {
                directory: true, ..
            }
            | Kind::SaveFiles(_) => "Select folder",
            Kind::Open { .. } => "Open",
            Kind::Save => "Save",
        };
        Ok(Self {
            kind,
            title,
            parent,
            name,
            filters,
            current_filter,
            choices,
            accept_label: string(&options, "accept_label")?
                .filter(|label| !label.is_empty())
                .unwrap_or_else(|| default_label.into()),
            activation_token: string(&options, "activation_token")?,
            modal: option::<bool>(&options, "modal")?.unwrap_or(true),
            folder: path(&options, "current_folder")?,
            file: path(&options, "current_file")?,
        })
    }
}

#[cfg(test)]
mod tests;
