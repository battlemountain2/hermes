// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::PathBuf;

use super::{CustomAction, CustomActionsFile};

fn action() -> CustomAction {
    CustomAction {
        name: "Edit".to_owned(),
        command: vec!["editor".to_owned(), "--".to_owned(), "{paths}".to_owned()],
        extensions: vec!["txt".to_owned(), ".MD".to_owned()],
        directories: false,
        multiple: true,
        confirm: false,
    }
}

#[test]
fn matches_extensions_case_insensitively() {
    assert!(action().applies_to(&[PathBuf::from("notes.TXT")]));
    assert!(action().applies_to(&[PathBuf::from("readme.md")]));
    assert!(!action().applies_to(&[PathBuf::from("image.png")]));
}

#[test]
fn paths_placeholder_expands_to_separate_arguments() {
    let paths = [PathBuf::from("one file.txt"), PathBuf::from("two.md")];
    let command = action().command_for(&paths).expect("valid command");
    assert_eq!(command.program, "editor");
    assert_eq!(command.arguments, ["--", "one file.txt", "two.md"]);
}

#[test]
fn single_placeholders_expand_without_a_shell() {
    let mut action = action();
    action.command = vec![
        "tool".to_owned(),
        "{path}".to_owned(),
        "{parent}".to_owned(),
        "{name}".to_owned(),
    ];
    let command = action
        .command_for(&[PathBuf::from("/tmp/folder/a file.txt")])
        .expect("valid command");
    assert_eq!(
        command.arguments,
        ["/tmp/folder/a file.txt", "/tmp/folder", "a file.txt"]
    );
}

#[test]
fn documented_toml_shape_deserializes() {
    let file: CustomActionsFile = toml::from_str(
        r#"
        [[actions]]
        name = "Optimize PNG"
        command = ["oxipng", "--", "{path}"]
        extensions = ["png"]
        multiple = false
        confirm = true
        "#,
    )
    .expect("documented custom action should parse");
    assert_eq!(file.actions.len(), 1);
    assert!(file.actions[0].confirm);
}
