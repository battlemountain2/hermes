// SPDX-License-Identifier: GPL-3.0-or-later

use std::{cell::Cell, rc::Rc};

use super::*;
use crate::{
    adapters::LocalFileSource,
    ui::browser::{BrowserView, PeekBehavior},
};

fn setup_test_gtk() {
    gtk::init().expect("initialize the isolated GTK display");
}

fn create_test_browser() -> BrowserView {
    BrowserView::new(Rc::new(LocalFileSource), PeekBehavior::default())
}

#[test]
fn test_active_pane_state_and_equality() {
    assert_eq!(ActivePane::Left, ActivePane::Left);
    assert_ne!(ActivePane::Left, ActivePane::Right);
}

#[test]
#[ignore = "Requires isolated GTK display; tests SplitView single pane mode"]
fn test_split_view_starts_in_single_pane_mode() {
    setup_test_gtk();
    let left = create_test_browser();
    let right = create_test_browser();
    let split = SplitView::new(left, right);

    assert!(!split.is_split());
    assert_eq!(split.active_pane(), ActivePane::Left);
}

#[test]
#[ignore = "Requires isolated GTK display; exercises SplitView dual pane toggle and switching"]
fn test_split_view_toggle_and_pane_switching() {
    setup_test_gtk();
    let left = create_test_browser();
    let right = create_test_browser();
    let split = SplitView::new(left, right);

    let toggled_state = Rc::new(Cell::new(false));
    let toggled_clone = toggled_state.clone();
    split.connect_split_toggled(move |split_active| {
        toggled_clone.set(split_active);
    });

    let active_pane_state = Rc::new(Cell::new(ActivePane::Left));
    let pane_clone = active_pane_state.clone();
    split.connect_active_pane_changed(move |pane| {
        pane_clone.set(pane);
    });

    // Toggle split on
    split.toggle_split();
    assert!(split.is_split());
    assert!(toggled_state.get());

    // Switch pane via Tab
    split.switch_pane();
    assert_eq!(split.active_pane(), ActivePane::Right);
    assert_eq!(active_pane_state.get(), ActivePane::Right);

    // Switch pane back
    split.switch_pane();
    assert_eq!(split.active_pane(), ActivePane::Left);
    assert_eq!(active_pane_state.get(), ActivePane::Left);

    // Toggle split off
    split.toggle_split();
    assert!(!split.is_split());
    assert!(!toggled_state.get());
    assert_eq!(split.active_pane(), ActivePane::Left);
}
