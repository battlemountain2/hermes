// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use std::{
    fs,
    time::{Duration, Instant},
};

#[test]
#[ignore = "Requires a private GTK display, bus and temporary stable XDG roots; initializes only generated stores"]
fn private_first_launch_waits_for_explicit_consent_before_importing() {
    assert!(
        !crate::paths::is_development(),
        "Run with private stable-profile roots"
    );
    let roots = Roots::current();
    let private_root =
        std::env::var_os("HERMES_SAFETY_TEST_ROOT").expect("Use the isolated safety-test runner");
    for path in roots.source.iter().chain(&roots.target) {
        assert!(
            path.starts_with(&private_root),
            "Private XDG roots are required"
        );
    }
    glib::set_prgname(Some("hermes-import-test"));
    gtk::init().unwrap();
    let application = gtk::Application::builder()
        .application_id("io.github.battlemountain2.Hermes.ImportTest")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    application.register(None::<&gio::Cancellable>).unwrap();
    fs::create_dir_all(&roots.source[0]).unwrap();
    let settings = b"[theme]\nmode = 'theme'\ntheme = 'graphite'\n";
    fs::write(roots.source[0].join("settings.toml"), settings).unwrap();
    let gate = Rc::new(Gate {
        application: application.downgrade(),
        phase: Cell::new(Phase::Idle),
        pending: RefCell::new(VecDeque::new()),
        window: RefCell::new(None),
    });
    fn until(mut done: impl FnMut() -> bool) {
        let start = Instant::now();
        while !done() {
            assert!(start.elapsed() < Duration::from_secs(8));
            while glib::MainContext::default().iteration(false) {}
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    gate.check();
    until(|| gate.phase.get() == Phase::Waiting);
    assert_eq!(application.windows().len(), 1);
    assert!(!roots.target[0].join("settings.toml").exists());
    assert!(!roots.target[2].join("legacy-import-v2").exists());
    let window = gate.window.borrow().as_ref().unwrap().clone();
    fn find_button(widget: &gtk::Widget) -> Option<gtk::Button> {
        if let Some(button) = widget.downcast_ref::<gtk::Button>() {
            if button.label().as_deref() == Some("Import / resume Strata data") {
                return Some(button.clone());
            }
        }
        let mut child = widget.first_child();
        while let Some(widget) = child {
            child = widget.next_sibling();
            if let Some(button) = find_button(&widget) {
                return Some(button);
            }
        }
        None
    }
    find_button(window.upcast_ref()).unwrap().emit_clicked();
    until(|| gate.phase.get() == Phase::Ready);
    assert_eq!(
        fs::read(roots.target[0].join("settings.toml")).unwrap(),
        settings
    );
    assert_eq!(
        fs::read(roots.source[0].join("settings.toml")).unwrap(),
        settings
    );
    assert!(gate.window.borrow().is_none());
    assert!(application.windows().is_empty());
}
