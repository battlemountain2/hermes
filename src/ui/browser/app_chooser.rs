#![expect(
    deprecated,
    reason = "GTK's direct application chooser provides MIME-aware grouping without a recursive portal"
)]

use gtk::{gio, glib, prelude::*};

pub(super) fn dialog(window: &gtk::Window, file: &gio::File) -> gtk::AppChooserDialog {
    let dialog = gtk::AppChooserDialog::new(Some(window), gtk::DialogFlags::MODAL, file);
    dialog.set_title(Some("Open With"));
    dialog.add_css_class("hermes-app-chooser");
    if let Some(chooser) = dialog.widget().downcast_ref::<gtk::AppChooserWidget>() {
        chooser.set_show_all(false);
        chooser.set_show_default(false);
        chooser.set_show_recommended(true);
        chooser.set_show_fallback(true);
        chooser.set_show_other(true);
    }
    dialog.connect_map(|dialog| {
        let chooser = dialog.widget().downgrade();
        glib::idle_add_local_once(move || {
            if let Some(chooser) = chooser.upgrade() {
                reset_scroll(&chooser);
            }
        });
    });
    dialog
}

fn reset_scroll(widget: &gtk::Widget) {
    if let Some(scroller) = widget.downcast_ref::<gtk::ScrolledWindow>() {
        let adjustment = scroller.vadjustment();
        adjustment.set_value(adjustment.lower());
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        reset_scroll(&widget);
    }
}

#[cfg(test)]
mod tests;
