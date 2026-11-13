// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

fn setup_test_gtk() {
    gtk::init().expect("initialize the isolated GTK display");
}

#[test]
#[ignore = "Requires isolated GTK display; verifies QuickLookModal initial closed state"]
fn test_quick_look_initial_state() {
    setup_test_gtk();
    let modal = QuickLookModal::new();
    assert!(!modal.is_open());
    modal.close();
    assert!(!modal.is_open());
}

#[test]
#[ignore = "Requires isolated GTK display; verifies QuickLookModal navigation handlers registration"]
fn test_quick_look_navigation_handlers() {
    setup_test_gtk();
    let modal = QuickLookModal::new();
    let next_called = Rc::new(Cell::new(false));
    let prev_called = Rc::new(Cell::new(false));

    let next_flag = next_called.clone();
    let prev_flag = prev_called.clone();
    modal.set_navigation_handlers(
        move || {
            next_flag.set(true);
            None
        },
        move || {
            prev_flag.set(true);
            None
        },
    );

    assert!(!next_called.get());
    assert!(!prev_called.get());
}
