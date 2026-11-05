use super::*;
use crate::services::{ConflictAction, ConflictReply, ConflictResolution};

pub(super) struct ConflictView {
    overlay: gtk::Overlay,
    layer: gtk::Box,
    reply: ConflictReply,
}

impl Drop for ConflictView {
    fn drop(&mut self) {
        self.reply.respond(ConflictResolution {
            action: ConflictAction::Cancel,
            apply_to_remaining: false,
        });
        if self.layer.parent().as_ref() == Some(self.overlay.upcast_ref()) {
            self.overlay.remove_overlay(&self.layer);
        }
    }
}

impl ViewState {
    pub(super) fn dismiss_transfer_conflict(&self) {
        self.transfer_conflict.borrow_mut().take();
    }

    pub(super) fn show_transfer_conflict(
        self: &Rc<Self>,
        source: FileEntry,
        existing: FileEntry,
        reply: ConflictReply,
    ) {
        self.dismiss_transfer_conflict();
        let Some(window) = self.overlay.root().and_downcast::<gtk::Window>() else {
            reply.respond(ConflictResolution {
                action: ConflictAction::Cancel,
                apply_to_remaining: false,
            });
            return;
        };
        let Some(overlay) = window.child().and_downcast::<gtk::Overlay>() else {
            reply.respond(ConflictResolution {
                action: ConflictAction::Cancel,
                apply_to_remaining: false,
            });
            return;
        };
        let merging = source.is_directory() && existing.is_directory();
        let mismatch = source.is_directory() != existing.is_directory();
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.add_css_class("delete-confirmation");
        content.add_css_class("delete-confirmation-content");
        content.set_halign(gtk::Align::Center);
        content.set_valign(gtk::Align::Center);
        let header = gtk::Box::new(gtk::Orientation::Vertical, 6);
        header.add_css_class("delete-confirmation-header");
        let title = gtk::Label::new(Some(if merging {
            "Merge these folders?"
        } else {
            "An item already exists"
        }));
        title.add_css_class("delete-confirmation-title");
        title.set_xalign(0.0);
        header.append(&title);
        content.append(&header);

        let body = gtk::Box::new(gtk::Orientation::Vertical, 12);
        body.add_css_class("delete-confirmation-body");
        let explanation = gtk::Label::new(Some(if merging {
            "Unrelated destination files will stay. You will choose separately for conflicting files inside these folders."
        } else if mismatch {
            "One item is a folder and the other is not. Keep both or skip to preserve their contents."
        } else {
            "Choose what happens to the existing destination file. Completed transfers will be kept if you cancel."
        }));
        explanation.add_css_class("delete-confirmation-explanation");
        explanation.set_wrap(true);
        explanation.set_max_width_chars(64);
        explanation.set_xalign(0.0);
        body.append(&explanation);
        let comparison = gtk::Box::new(
            if window.width() >= 780 {
                gtk::Orientation::Horizontal
            } else {
                gtk::Orientation::Vertical
            },
            10,
        );
        comparison.add_css_class("conflict-comparison-row");
        comparison.append(&card("Incoming", &source));
        if comparison.orientation() == gtk::Orientation::Horizontal {
            comparison.append(&crate::assets::primary_icon(
                crate::assets::icons::ARROW_RIGHT,
                18,
            ));
        }
        comparison.append(&card("Existing destination", &existing));
        body.append(&comparison);
        let apply = gtk::CheckButton::with_label(if merging {
            "Apply to remaining folder merges in this transfer"
        } else if mismatch {
            "Apply to remaining type conflicts in this transfer"
        } else {
            "Apply to remaining file conflicts in this transfer"
        });
        apply.add_css_class("collision-apply-all");
        body.append(&apply);
        content.append(&body);
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        actions.add_css_class("delete-confirmation-actions");
        let layer = modal_layer(&content);
        for (label, action) in [
            ("Cancel", ConflictAction::Cancel),
            ("Skip", ConflictAction::Skip),
            ("Keep both", ConflictAction::KeepBoth),
            (
                if merging { "Merge" } else { "Replace" },
                ConflictAction::Replace,
            ),
        ] {
            let button = gtk::Button::with_label(label);
            button.add_css_class(if action == ConflictAction::Replace && !merging {
                "delete-confirmation-delete"
            } else {
                "delete-confirmation-cancel"
            });
            button.set_sensitive(action != ConflictAction::Replace || !mismatch);
            let weak = Rc::downgrade(self);
            let response = reply.clone();
            let apply = apply.clone();
            button.connect_clicked(move |_| {
                response.respond(ConflictResolution {
                    action,
                    apply_to_remaining: apply.is_active(),
                });
                if let Some(state) = weak.upgrade() {
                    state.dismiss_transfer_conflict();
                }
            });
            actions.append(&button);
        }
        content.append(&actions);
        let escape = gtk::EventControllerKey::new();
        let weak = Rc::downgrade(self);
        escape.connect_key_pressed(move |_, key, _, _| {
            if key == gtk::gdk::Key::Escape {
                if let Some(state) = weak.upgrade() {
                    state.dismiss_transfer_conflict();
                }
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
        layer.add_controller(escape);
        overlay.add_overlay(&layer);
        self.transfer_conflict.replace(Some(ConflictView {
            overlay,
            layer,
            reply,
        }));
        if let Some(button) = actions.first_child() {
            button.grab_focus();
        }
    }
}

fn card(heading: &str, entry: &FileEntry) -> gtk::Box {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 6);
    card.add_css_class("conflict-card");
    card.set_hexpand(true);
    let heading = gtk::Label::new(Some(heading));
    heading.add_css_class("conflict-card-heading");
    heading.set_xalign(0.0);
    card.append(&heading);
    let name = gtk::Label::new(Some(&entry.display_name));
    name.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    name.set_max_width_chars(28);
    name.set_xalign(0.0);
    name.add_css_class("conflict-card-filename");
    card.append(&name);
    let metadata = gtk::Label::new(Some(&format!(
        "{} · {}",
        match entry.size {
            crate::model::MetadataValue::Known(bytes) => formatting::format_size(bytes),
            _ => if entry.is_directory() {
                "Folder"
            } else {
                "Size unknown"
            }
            .to_owned(),
        },
        match entry.modified_unix_seconds {
            crate::model::MetadataValue::Known(time) => formatting::format_timestamp(time),
            _ => "Date unknown".to_owned(),
        }
    )));
    metadata.set_wrap(true);
    metadata.set_xalign(0.0);
    metadata.add_css_class("conflict-card-meta");
    card.append(&metadata);
    let path = gtk::Label::new(Some(&entry.location.display_path()));
    path.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    path.set_max_width_chars(28);
    path.set_xalign(0.0);
    path.add_css_class("conflict-card-meta");
    path.set_tooltip_text(Some(&entry.location.display_path()));
    card.append(&path);
    card
}

#[cfg(test)]
mod tests;
