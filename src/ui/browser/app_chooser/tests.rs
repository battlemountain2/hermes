use super::*;
use std::{
    fs,
    time::{Duration, Instant},
};

fn application_names(widget: &gtk::Widget) -> Option<Vec<String>> {
    if let Some(list) = widget.downcast_ref::<gtk::ListView>() {
        let model = list.model()?;
        return Some(
            (0..model.n_items())
                .filter_map(|position| model.item(position))
                .map(|item| item.property::<String>("name"))
                .collect(),
        );
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if let Some(names) = application_names(&widget) {
            return Some(names);
        }
    }
    None
}

fn check_scroll_position(widget: &gtk::Widget) {
    if let Some(scroller) = widget.downcast_ref::<gtk::ScrolledWindow>() {
        let adjustment = scroller.vadjustment();
        assert!(
            (adjustment.value() - adjustment.lower()).abs() < 1.0,
            "Chooser did not open at the suggested apps"
        );
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        check_scroll_position(&widget);
    }
}

#[test]
#[ignore = "Private GTK and isolated HOME/XDG fixture only; never launches apps or changes real associations"]
fn suggested_apps_precede_unrelated_apps_without_launching_or_changing_defaults() {
    let root = std::env::var_os("HERMES_PICKER_TEST_ROOT")
        .map(std::path::PathBuf::from)
        .expect("Run with the private picker harness");
    assert_eq!(gtk::glib::user_data_dir(), root.join("data"));
    assert_eq!(gtk::glib::user_config_dir(), root.join("config"));
    let applications = root.join("data/applications");
    fs::create_dir_all(&applications).unwrap();
    for (id, name, mime) in [
        ("hermes-test-default", "Zebra Default Player", "audio/flac;"),
        (
            "hermes-test-suggested",
            "Middle Suggested Player",
            "audio/flac;",
        ),
        ("hermes-test-other", "Aardvark Other Editor", "text/plain;"),
    ] {
        fs::write(applications.join(format!("{id}.desktop")), format!("[Desktop Entry]\nType=Application\nName={name}\nExec=/bin/false %f\nMimeType={mime}\n")).unwrap();
    }
    fs::write(root.join("config/mimeapps.list"), "[Default Applications]\naudio/flac=hermes-test-default.desktop;\n[Added Associations]\naudio/flac=hermes-test-default.desktop;hermes-test-suggested.desktop;\n").unwrap();
    let file = root.join("home/generated.flac");
    fs::write(&file, b"fLaC").unwrap();
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    super::super::super::window::load_styles();
    let _theme = super::super::super::theme::ThemeManager::shared();
    let window = gtk::Window::new();
    let dialog = dialog(&window, &gio::File::for_path(&file));
    let chooser = dialog.widget().downcast::<gtk::AppChooserWidget>().unwrap();
    assert!(!chooser.shows_all());
    assert!(!chooser.shows_default());
    assert!(chooser.shows_recommended());
    assert!(chooser.shows_other());
    dialog.present();
    let settled = Instant::now() + Duration::from_millis(160);
    while Instant::now() < settled {
        gtk::glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
    if let Some(path) = std::env::var_os("HERMES_PICKER_SCREENSHOT") {
        let paintable = gtk::WidgetPaintable::new(Some(&dialog));
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(
            &snapshot,
            f64::from(dialog.width()),
            f64::from(dialog.height()),
        );
        if let Some(node) = snapshot.to_node() {
            let renderer = gtk::gsk::CairoRenderer::new();
            renderer.realize(None::<&gtk::gdk::Surface>).unwrap();
            renderer
                .render_texture(&node, None)
                .save_to_png(std::path::PathBuf::from(path).with_file_name("open-with.png"))
                .unwrap();
            renderer.unrealize();
        }
    }
    let deadline = Instant::now() + Duration::from_secs(5);
    let values = loop {
        gtk::glib::MainContext::default().iteration(false);
        let values = application_names(chooser.upcast_ref()).unwrap_or_default();
        if values.iter().any(|value| value == "Aardvark Other Editor") {
            break values;
        }
        assert!(
            Instant::now() < deadline,
            "Missing fixture application rows: {values:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    };
    let position = |name| {
        values
            .iter()
            .position(|value| value == name)
            .unwrap_or_else(|| panic!("Missing {name} in {values:?}"))
    };
    assert!(position("Zebra Default Player") < position("Aardvark Other Editor"));
    assert!(position("Middle Suggested Player") < position("Aardvark Other Editor"));
    check_scroll_position(chooser.upcast_ref());
    let selected = chooser.app_info().unwrap();
    assert!(
        gio::AppInfo::recommended_for_type("audio/flac")
            .iter()
            .any(|app| app.equal(&selected))
    );
    dialog.close();
    window.destroy();
    assert_eq!(
        gio::AppInfo::default_for_type("audio/flac", false)
            .unwrap()
            .id()
            .as_deref(),
        Some("hermes-test-default.desktop")
    );
}
