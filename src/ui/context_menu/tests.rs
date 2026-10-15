use super::*;

#[test]
fn defaults_keep_clipboard_actions_primary_and_optional_dialogs_hidden() {
    let defaults = Options::default();
    assert!(defaults.archives && defaults.music && defaults.open_with);
    assert!(!defaults.transfer_destinations && !defaults.compress_files);
    let restored: Options = toml::from_str("music = false\ntransfer_destinations = true").unwrap();
    assert!(!restored.music);
    assert!(restored.transfer_destinations && restored.archives);
    assert_eq!(
        toml::from_str::<Options>(&toml::to_string(&restored).unwrap()).unwrap(),
        restored
    );
}

#[test]
#[ignore = "Requires an isolated GTK display; no personal files"]
fn desktop_context_sections_hide_empty_and_duplicate_separators() {
    gtk::init().unwrap();
    let container = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let separators: Vec<_> = (0..4)
        .map(|_| gtk::Separator::new(gtk::Orientation::Horizontal))
        .collect();
    let first = gtk::Button::with_label("Copy");
    let optional = gtk::Button::with_label("Copy to");
    optional.set_visible(false);
    let last = gtk::Button::with_label("Properties");
    container.append(&separators[0]);
    container.append(&first);
    container.append(&separators[1]);
    container.append(&optional);
    container.append(&separators[2]);
    container.append(&last);
    container.append(&separators[3]);
    tidy_separators(&container);
    assert!(!separators[0].is_visible() && !separators[3].is_visible());
    assert!(separators[1].is_visible() && !separators[2].is_visible());
    optional.set_visible(true);
    tidy_separators(&container);
    assert!(separators[1].is_visible() && separators[2].is_visible());
    let hidden_parent = gtk::Box::new(gtk::Orientation::Vertical, 0);
    hidden_parent.append(&container);
    hidden_parent.set_visible(false);
    tidy_separators(&container);
    assert!(separators[1].get_visible() && separators[2].get_visible());
}
