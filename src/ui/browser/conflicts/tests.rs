use super::*;

#[test]
#[ignore = "Private GTK display and isolated XDG roots; verifies real conflict buttons"]
fn conflict_dialog_returns_the_chosen_policy_and_cancels_when_dismissed() {
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    crate::ui::window::load_styles();
    let _theme = crate::ui::theme::ThemeManager::shared();
    let view = BrowserView::new(
        Rc::new(crate::adapters::LocalFileSource),
        super::super::PeekBehavior::default(),
    );
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&view.widget()));
    let window = gtk::Window::builder()
        .child(&overlay)
        .default_width(900)
        .default_height(700)
        .build();
    window.present();
    let mut source = crate::app::music_order::tests::entry("a < b & c.txt", None, None);
    Rc::make_mut(&mut source.0).kind = EntryKind::Directory;
    let reply = ConflictReply::default();
    view.state
        .show_transfer_conflict(source.clone(), source.clone(), reply.clone());
    fn button(widget: &gtk::Widget, label: &str) -> Option<gtk::Button> {
        if let Some(button) = widget.downcast_ref::<gtk::Button>()
            && button.label().as_deref() == Some(label)
        {
            return Some(button.clone());
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            child = current.next_sibling();
            if let Some(found) = button(&current, label) {
                return Some(found);
            }
        }
        None
    }
    button(overlay.upcast_ref(), "Merge")
        .unwrap()
        .emit_clicked();
    assert_eq!(
        glib::MainContext::default().block_on(reply.wait()),
        ConflictResolution {
            action: ConflictAction::Replace,
            apply_to_remaining: false
        }
    );
    assert!(view.state.transfer_conflict.borrow().is_none());
    let dismissed = ConflictReply::default();
    view.state
        .show_transfer_conflict(source.clone(), source, dismissed.clone());
    view.state.dismiss_transfer_conflict();
    assert_eq!(
        glib::MainContext::default()
            .block_on(dismissed.wait())
            .action,
        ConflictAction::Cancel
    );
    view.state.browser.clear_observer();
    window.destroy();
}
