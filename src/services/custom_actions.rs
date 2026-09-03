// SPDX-License-Identifier: GPL-3.0-or-later

use std::{ffi::OsStr, fs, path::PathBuf};

use serde::Deserialize;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct CustomAction {
    pub name: String,
    pub command: Vec<String>,
    #[serde(default)]
    pub extensions: Vec<String>,
    #[serde(default)]
    pub directories: bool,
    #[serde(default)]
    pub multiple: bool,
    #[serde(default)]
    pub confirm: bool,
}

#[derive(Debug, Default, Deserialize)]
struct CustomActionsFile {
    #[serde(default)]
    actions: Vec<CustomAction>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CustomCommand {
    pub program: String,
    pub arguments: Vec<String>,
}

pub fn actions_path() -> PathBuf {
    gtk::glib::user_config_dir().join("hermes/actions.toml")
}

pub fn load_custom_actions() -> Result<Vec<CustomAction>, String> {
    let path = actions_path();
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let contents = fs::read_to_string(&path)
        .map_err(|error| format!("Unable to read {}: {error}", path.display()))?;
    let file: CustomActionsFile = toml::from_str(&contents)
        .map_err(|error| format!("Unable to parse {}: {error}", path.display()))?;
    for action in &file.actions {
        if action.name.trim().is_empty() || action.command.is_empty() {
            return Err("Every custom action needs a name and command".to_owned());
        }
    }
    let mut actions = file.actions;
    actions.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(actions)
}

pub fn create_custom_actions_template() -> Result<PathBuf, String> {
    let path = actions_path();
    if path.exists() {
        return Ok(path);
    }
    let Some(parent) = path.parent() else {
        return Err("Unable to determine the custom-actions directory".to_owned());
    };
    fs::create_dir_all(parent)
        .map_err(|error| format!("Unable to create {}: {error}", parent.display()))?;
    fs::write(&path, TEMPLATE)
        .map_err(|error| format!("Unable to create {}: {error}", path.display()))?;
    Ok(path)
}

impl CustomAction {
    pub fn applies_to(&self, paths: &[PathBuf]) -> bool {
        if paths.is_empty() || (paths.len() > 1 && !self.multiple) {
            return false;
        }
        paths.iter().all(|path| {
            if path.is_dir() {
                return self.directories;
            }
            self.extensions.is_empty()
                || path
                    .extension()
                    .and_then(OsStr::to_str)
                    .is_some_and(|extension| {
                        self.extensions.iter().any(|allowed| {
                            allowed
                                .trim_start_matches('.')
                                .eq_ignore_ascii_case(extension)
                        })
                    })
        })
    }

    pub fn command_for(&self, paths: &[PathBuf]) -> Result<CustomCommand, String> {
        if !self.applies_to(paths) {
            return Err("This action does not support the current selection".to_owned());
        }
        let first = &paths[0];
        let replace = |value: &str| {
            value
                .replace("{path}", &first.to_string_lossy())
                .replace(
                    "{parent}",
                    &first.parent().unwrap_or(first).to_string_lossy(),
                )
                .replace(
                    "{name}",
                    &first.file_name().unwrap_or_default().to_string_lossy(),
                )
        };
        let mut values = Vec::new();
        for value in &self.command {
            if value == "{paths}" {
                values.extend(paths.iter().map(|path| path.to_string_lossy().into_owned()));
            } else {
                values.push(replace(value));
            }
        }
        let Some((program, arguments)) = values.split_first() else {
            return Err("The custom action has no command".to_owned());
        };
        if program.trim().is_empty() {
            return Err("The custom action has no executable".to_owned());
        }
        Ok(CustomCommand {
            program: program.clone(),
            arguments: arguments.to_vec(),
        })
    }
}

const TEMPLATE: &str = r#"# Hermes custom actions
# Each command is an argument list and is never passed through a shell.
# Placeholders: {path}, {paths}, {parent}, and {name}.

[[actions]]
name = "Open terminal here"
command = ["foot", "--working-directory", "{path}"]
directories = true

[[actions]]
name = "Edit with Kate"
command = ["kate", "{paths}"]
extensions = ["txt", "md", "toml", "rs"]
multiple = true
"#;

#[cfg(test)]
mod tests;
