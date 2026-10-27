use super::*;
use std::cell::Cell;

#[test]
#[ignore = "Private GTK/preferences; verifies explicit opt-in without any network or playback"]
fn desktop_online_lyrics_requires_explicit_opt_in_and_weakly_notifies() {
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let manager = ThemeManager::shared();
    assert!(!manager.online_lyrics());
    let page = audio_page(manager.clone());
    let toggle = option_control(&page, "Look up missing lyrics online")
        .unwrap()
        .downcast::<gtk::Switch>()
        .unwrap();
    assert!(!toggle.is_active());
    let calls = Rc::new(Cell::new(0));
    let count = calls.clone();
    let callback: Rc<dyn Fn()> = Rc::new(move || count.set(count.get() + 1));
    manager.observe_online_lyrics(&callback);
    manager.observe_online_lyrics(&callback);
    toggle.set_active(true);
    assert!(manager.online_lyrics());
    assert_eq!(calls.get(), 1);
    manager.set_online_lyrics(true);
    assert_eq!(calls.get(), 1);
    let saved: toml::Value = toml::from_str(
        &std::fs::read_to_string(crate::paths::settings_file())
            .unwrap(),
    )
    .unwrap();
    assert_eq!(saved["online_lyrics"].as_bool(), Some(true));
    let reopened = audio_page(manager.clone());
    assert!(
        option_control(&reopened, "Look up missing lyrics online")
            .unwrap()
            .downcast::<gtk::Switch>()
            .unwrap()
            .is_active()
    );
    drop(callback);
    toggle.set_active(false);
    assert!(!manager.online_lyrics());
    assert_eq!(calls.get(), 1);
}

fn option_control(widget: &gtk::Widget, title: &str) -> Option<gtk::Widget> {
    if widget
        .first_child()
        .and_then(|copy| copy.first_child())
        .and_downcast::<gtk::Label>()
        .is_some_and(|label| label.text() == title)
    {
        return widget.last_child();
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        if let Some(control) = option_control(&current, title) {
            return Some(control);
        }
    }
    None
}

#[test]
#[ignore = "Requires isolated GTK display and private HOME/XDG preferences; no decoder"]
fn desktop_audio_choices_apply_live_persist_and_reopen_without_touching_playback() {
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let manager = ThemeManager::shared();
    manager.set_audio_spin_animation(true);
    manager.set_audio_spectrum_bands(32);
    manager.set_audio_spectrum_decay("studio");
    let page = audio_page(manager.clone());
    let spin = option_control(&page, "Spinning record in audio previews")
        .unwrap()
        .downcast::<gtk::Switch>()
        .unwrap();
    let columns = option_control(&page, "Preview visualizer columns")
        .unwrap()
        .downcast::<gtk::DropDown>()
        .unwrap();
    let decay = option_control(&page, "Ballistics & meter falloff")
        .unwrap()
        .downcast::<gtk::DropDown>()
        .unwrap();
    assert!(spin.is_sensitive() && columns.is_sensitive() && decay.is_sensitive());
    assert_eq!(columns.model().unwrap().n_items(), 2);
    assert!(option_control(&page, "Background playback continuity").is_none());
    let controller = crate::ui::test_music_controller();
    let initial_status = controller.status();
    let calls = Rc::new(Cell::new(0));
    let count = calls.clone();
    let observer: Rc<dyn Fn()> = Rc::new(move || count.set(count.get() + 1));
    super::super::audio_preferences::observe(&observer);
    spin.set_active(false);
    columns.set_selected(0);
    decay.set_selected(2);
    assert_eq!(calls.get(), 3);
    assert!(!super::super::audio_preferences::current().spin);
    assert_eq!(super::super::audio_preferences::current().columns, 16);
    assert_eq!(
        super::super::audio_preferences::current().decay.id(),
        "smooth"
    );
    assert_eq!(controller.status(), initial_status);
    manager.set_audio_spectrum_bands(64);
    manager.set_audio_spectrum_decay("invalid");
    assert_eq!(manager.audio_spectrum_bands(), 16);
    assert_eq!(manager.audio_spectrum_decay(), "smooth");
    assert_eq!(calls.get(), 3);
    let saved: toml::Value = toml::from_str(
        &std::fs::read_to_string(crate::paths::settings_file())
            .unwrap(),
    )
    .unwrap();
    assert_eq!(saved["audio_spin_animation"].as_bool(), Some(false));
    assert_eq!(saved["audio_spectrum_bands"].as_integer(), Some(16));
    assert_eq!(saved["audio_spectrum_decay"].as_str(), Some("smooth"));
    let reopened = audio_page(manager.clone());
    assert!(
        !option_control(&reopened, "Spinning record in audio previews")
            .unwrap()
            .downcast::<gtk::Switch>()
            .unwrap()
            .is_active()
    );
    assert_eq!(
        option_control(&reopened, "Preview visualizer columns")
            .unwrap()
            .downcast::<gtk::DropDown>()
            .unwrap()
            .selected(),
        0
    );
    assert_eq!(
        option_control(&reopened, "Ballistics & meter falloff")
            .unwrap()
            .downcast::<gtk::DropDown>()
            .unwrap()
            .selected(),
        2
    );
    columns.set_selected(1);
    decay.set_selected(1);
    assert_eq!(super::super::audio_preferences::current().columns, 32);
    assert_eq!(
        super::super::audio_preferences::current().decay.id(),
        "snappy"
    );
}

#[test]
#[ignore = "Requires isolated GTK display and private HOME/XDG preferences"]
fn desktop_date_and_size_choices_update_live_labels_and_save_validated_values() {
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let manager = ThemeManager::shared();
    manager.set_date_format("relative");
    manager.set_size_unit("decimal");
    let browser = BrowserView::new(Rc::new(EmptySource), Default::default());
    let page = view_formats_page(&browser, manager.clone(), Rc::new(|| {}));
    let date = option_control(&page, "Timestamp presentation")
        .unwrap()
        .downcast::<gtk::DropDown>()
        .unwrap();
    let unit = option_control(&page, "Byte unit prefixes")
        .unwrap()
        .downcast::<gtk::DropDown>()
        .unwrap();
    let sample = option_control(&page, "Live file size example")
        .unwrap()
        .downcast::<gtk::Label>()
        .unwrap();
    assert!(date.is_sensitive() && unit.is_sensitive());
    assert_eq!(sample.text(), "1.4 GB");
    let size_label = gtk::Label::new(None);
    super::super::formatting::bind(
        &size_label,
        super::super::formatting::Value::Size(1_500_000),
    );
    assert_eq!(size_label.text(), "1.5 MB");
    unit.set_selected(1);
    assert_eq!(manager.size_unit(), "binary");
    assert_eq!(size_label.text(), "1.4 MiB");
    assert_eq!(sample.text(), "1.3 GiB");
    date.set_selected(1);
    assert_eq!(manager.date_format(), "iso");
    manager.set_date_format("unsupported");
    manager.set_size_unit("unsupported");
    assert_eq!(manager.date_format(), "iso");
    assert_eq!(manager.size_unit(), "binary");
    let saved: toml::Value = toml::from_str(
        &std::fs::read_to_string(crate::paths::settings_file())
            .unwrap(),
    )
    .unwrap();
    assert_eq!(saved["date_format"].as_str(), Some("iso"));
    assert_eq!(saved["size_unit"].as_str(), Some("binary"));
    let reopened = view_formats_page(&browser, manager.clone(), Rc::new(|| {}));
    assert_eq!(
        option_control(&reopened, "Timestamp presentation")
            .unwrap()
            .downcast::<gtk::DropDown>()
            .unwrap()
            .selected(),
        1
    );
    assert_eq!(
        option_control(&reopened, "Byte unit prefixes")
            .unwrap()
            .downcast::<gtk::DropDown>()
            .unwrap()
            .selected(),
        1
    );
    unit.set_selected(0);
    date.set_selected(0);
    assert_eq!(size_label.text(), "1.5 MB");
    assert_eq!(sample.text(), "1.4 GB");
    browser.browser().clear_observer();
}

struct EmptySource;

impl crate::services::FileSource for EmptySource {
    fn validate_location(
        &self,
        _: &crate::model::Location,
    ) -> Result<(), crate::services::LocationValidationError> {
        Ok(())
    }

    fn enumerate(
        &self,
        request: crate::services::DirectoryRequest,
        emit: Rc<dyn Fn(crate::services::DirectoryEvent)>,
    ) -> crate::services::LoadHandle {
        emit(crate::services::DirectoryEvent::Finished {
            request_id: request.id,
        });
        crate::services::LoadHandle::new(|| {})
    }
}

fn option_switch(widget: &gtk::Widget, title: &str) -> Option<gtk::Switch> {
    if widget
        .first_child()
        .and_then(|copy| copy.first_child())
        .and_downcast::<gtk::Label>()
        .is_some_and(|label| label.text() == title)
    {
        if let Some(toggle) = widget.last_child().and_downcast::<gtk::Switch>() {
            return Some(toggle);
        }
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        if let Some(toggle) = option_switch(&current, title) {
            return Some(toggle);
        }
    }
    None
}

#[test]
#[ignore = "Requires isolated GTK display and private HOME/XDG settings"]
fn desktop_general_preferences_are_live_and_saved_without_changing_tabs() {
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let manager = ThemeManager::shared();
    let browser = BrowserView::new(Rc::new(EmptySource), Default::default());
    browser
        .browser()
        .navigate(crate::model::Location::local("/generated-settings"));
    let general = general_page(&browser, manager.clone(), Rc::new(|| {}));
    let startup = general.first_child().unwrap().next_sibling().unwrap();
    let mut button = startup.first_child();
    for location in ["home", "music", "last"] {
        let current = button.take().unwrap().downcast::<gtk::Button>().unwrap();
        assert!(current.is_sensitive());
        current.emit_clicked();
        assert_eq!(manager.startup_location(), location);
        assert!(current.has_css_class("selected"));
        let mut other = startup.first_child();
        while let Some(candidate) = other {
            other = candidate.next_sibling();
            assert_eq!(candidate.has_css_class("selected"), candidate == current);
        }
        button = current.next_sibling();
    }
    assert_eq!(
        browser.browser().active_location(),
        Some(crate::model::Location::local("/generated-settings"))
    );
    let confirm = option_switch(&general, "Confirm permanent deletion").unwrap();
    confirm.set_active(false);
    assert!(!manager.confirm_permanent_delete());
    confirm.set_active(true);
    assert!(manager.confirm_permanent_delete());
    let motion = option_switch(&general, "Reduce motion").unwrap();
    motion.set_active(true);
    assert!(manager.reduce_motion());
    assert!(!super::super::motion::animations_enabled());
    let reopened = general_page(&browser, manager.clone(), Rc::new(|| {}));
    assert!(
        option_switch(&reopened, "Reduce motion")
            .unwrap()
            .is_active()
    );
    let formats = view_formats_page(&browser, manager.clone(), Rc::new(|| {}));
    let hidden = option_switch(&formats, "Show hidden files by default").unwrap();
    hidden.set_active(true);
    assert!(manager.show_hidden_default());
    assert!(browser.browser().column_preferences(0).unwrap().show_hidden);
    let fresh = BrowserView::new(Rc::new(EmptySource), Default::default());
    fresh
        .browser()
        .navigate(crate::model::Location::local("/generated-fresh"));
    assert!(fresh.browser().column_preferences(0).unwrap().show_hidden);
    let mut restored = fresh.browser().column_preferences(0).unwrap();
    restored.show_hidden = false;
    fresh.browser().set_preferences(restored);
    assert!(!fresh.browser().column_preferences(0).unwrap().show_hidden);
    assert!(manager.show_hidden_default());
    let saved: toml::Value = toml::from_str(
        &std::fs::read_to_string(crate::paths::settings_file())
            .unwrap(),
    )
    .unwrap();
    assert_eq!(saved["startup_location"].as_str(), Some("last"));
    assert_eq!(saved["reduce_motion"].as_bool(), Some(true));
    assert_eq!(saved["show_hidden_default"].as_bool(), Some(true));
    assert_eq!(saved["confirm_permanent_delete"].as_bool(), Some(true));
    hidden.set_active(false);
    assert!(!browser.browser().column_preferences(0).unwrap().show_hidden);
    assert!(!manager.show_hidden_default());
    browser.browser().clear_observer();
    fresh.browser().clear_observer();
}
