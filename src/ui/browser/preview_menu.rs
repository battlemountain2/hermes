use super::*;
use crate::ui::preview::{ImageContextMenuHandler, ImageMenuActions};

impl BrowserView {
    pub(in crate::ui) fn image_context_menu_handler(&self) -> ImageContextMenuHandler {
        let state = Rc::downgrade(&self.state);
        Rc::new(move |anchor, entry, actions, horizontal, vertical| {
            if state.upgrade().is_some() {
                show(anchor, state.clone(), entry, actions, horizontal, vertical);
            }
        })
    }
}

fn dismiss(anchor: &gtk::Widget) {
    let mut child = anchor.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if let Ok(menu) = widget.downcast::<gtk::Popover>()
            && menu.has_css_class("image-context-menu")
        {
            dismiss_menu(&menu);
        }
    }
}

fn dismiss_menu(menu: &gtk::Popover) {
    menu.popdown();
    if menu.parent().is_some() {
        menu.unparent();
    }
}

fn show(
    anchor: &gtk::Widget,
    state: std::rc::Weak<ViewState>,
    entry: FileEntry,
    actions: ImageMenuActions,
    horizontal: f64,
    vertical: f64,
) -> gtk::Popover {
    dismiss(anchor);
    let menu = gtk::Popover::new();
    menu.add_css_class("folder-context-popover");
    menu.add_css_class("image-context-menu");
    menu.set_has_arrow(false);
    menu.set_parent(anchor);
    menu.set_pointing_to(Some(&gtk::gdk::Rectangle::new(
        horizontal as i32,
        vertical as i32,
        1,
        1,
    )));
    let content = gtk::Box::new(gtk::Orientation::Vertical, 2);
    content.add_css_class("item-context-menu");
    let header = gtk::Box::new(gtk::Orientation::Vertical, 2);
    header.add_css_class("item-context-header");
    let title = gtk::Label::new(Some(&entry.display_name));
    title.add_css_class("item-context-title");
    title.set_xalign(0.0);
    title.set_max_width_chars(32);
    title.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    title.set_tooltip_text(Some(&entry.location.display_path()));
    header.append(&title);
    content.append(&header);
    let in_trash = is_trash_location(&entry.location);
    let options = crate::ui::theme::ThemeManager::shared().context_menu_options();
    for (action, icon, label, shortcut) in [
        (
            FileAction::Open,
            crate::assets::icons::EXTERNAL_LINK,
            "Open",
            "↵",
        ),
        (
            FileAction::OpenWith,
            crate::assets::icons::EXTERNAL_LINK,
            "Open With…",
            "",
        ),
        (
            FileAction::Cut,
            crate::assets::icons::SCISSORS,
            "Cut",
            "Ctrl+X",
        ),
        (
            FileAction::Copy,
            crate::assets::icons::COPY,
            "Copy",
            "Ctrl+C",
        ),
        (
            FileAction::CopyLocation,
            crate::assets::icons::COPY,
            "Copy location",
            "Y",
        ),
        (
            FileAction::Rename,
            crate::assets::icons::PENCIL,
            "Rename",
            "F2",
        ),
        (
            FileAction::Trash,
            crate::assets::icons::TRASH,
            if in_trash {
                "Delete Permanently"
            } else {
                "Move to Trash"
            },
            "Del",
        ),
        (
            FileAction::Properties,
            crate::assets::icons::INFO,
            "Properties",
            "Alt+Enter",
        ),
    ] {
        if matches!(action, FileAction::OpenWith) && (!options.open_with || in_trash) {
            continue;
        }
        if matches!(action, FileAction::Cut | FileAction::Properties) {
            content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        }
        let button = if matches!(action, FileAction::Trash) {
            let button = item_context_danger_option(icon, label, shortcut);
            button.add_css_class("danger");
            button
        } else {
            item_context_option(icon, label, shortcut)
        };
        if matches!(action, FileAction::Cut | FileAction::Rename) {
            button.set_sensitive(!in_trash);
        }
        let target = entry.clone();
        let state = state.clone();
        let weak_anchor = anchor.downgrade();
        let weak_menu = menu.downgrade();
        let fullscreen = actions.fullscreen.clone();
        let exiting_fullscreen = actions.exiting_fullscreen;
        button.connect_clicked(move |_| {
            if let Some(menu) = weak_menu.upgrade() {
                dismiss_menu(&menu);
            }
            let Some(state) = state.upgrade() else { return };
            if exiting_fullscreen && matches!(action, FileAction::Trash | FileAction::Properties) {
                if let Some(exit) = fullscreen.upgrade() {
                    exit.emit_clicked();
                }
                if let Some(window) = state.overlay.root().and_downcast::<gtk::Window>() {
                    window.present();
                }
            }
            match action {
                FileAction::Open => {
                    if let Some(anchor) = weak_anchor.upgrade() {
                        open_location(&target.location, &anchor);
                    }
                }
                FileAction::OpenWith => {
                    if let Some(anchor) = weak_anchor.upgrade() {
                        open_with(&anchor, &target);
                    }
                }
                FileAction::Cut => state.cut_entries(std::slice::from_ref(&target)),
                FileAction::Copy => state.copy_entries(std::slice::from_ref(&target)),
                FileAction::CopyLocation => copy_locations(std::slice::from_ref(&target)),
                FileAction::Rename => {
                    if let Some(anchor) = weak_anchor.upgrade() {
                        rename(&anchor, &state, target.clone());
                    }
                }
                FileAction::Trash => state.show_delete_confirmation(vec![target.clone()], in_trash),
                FileAction::Properties => state.show_entry_properties(target.clone()),
            }
        });
        content.append(&button);
    }
    content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    for (icon, label, shortcut, target) in [
        (
            crate::assets::icons::MAXIMIZE,
            if actions.exiting_fullscreen {
                "Exit fullscreen"
            } else {
                "Fullscreen"
            },
            "F",
            actions.fullscreen,
        ),
        (crate::assets::icons::MINIMIZE, "Fit", "", actions.fit),
        (crate::assets::icons::EYE, "100%", "", actions.actual),
    ] {
        let button = item_context_option(icon, label, shortcut);
        let weak_menu = menu.downgrade();
        button.connect_clicked(move |_| {
            if let Some(menu) = weak_menu.upgrade() {
                dismiss_menu(&menu);
            }
            if let Some(target) = target.upgrade() {
                target.emit_clicked();
            }
        });
        content.append(&button);
    }
    crate::ui::context_menu::tidy_separators(&content);
    menu.set_child(Some(&content));
    menu.connect_closed(|menu| {
        if menu.parent().is_some() {
            menu.unparent();
        }
    });
    menu.popup();
    menu
}

#[derive(Clone, Copy)]
enum FileAction {
    Open,
    OpenWith,
    Cut,
    Copy,
    CopyLocation,
    Rename,
    Trash,
    Properties,
}

#[expect(deprecated, reason = "Reuse the application's GTK app chooser")]
fn open_with(anchor: &gtk::Widget, entry: &FileEntry) {
    let Some(window) = anchor.root().and_downcast::<gtk::Window>() else {
        return;
    };
    let file = gio_file_for_location(&entry.location);
    let dialog = app_chooser::dialog(&window, &file);
    dialog.connect_response(move |dialog, response| {
        if response == gtk::ResponseType::Accept
            && let Some(app) = dialog.app_info()
            && let Err(error) =
                app.launch(std::slice::from_ref(&file), None::<&gio::AppLaunchContext>)
        {
            tracing::warn!(%error, "unable to launch selected application");
        }
        dialog.close();
    });
    dialog.present();
}

#[expect(
    deprecated,
    reason = "Reuse the application's themed rename dialog controls"
)]
fn rename(anchor: &gtk::Widget, state: &Rc<ViewState>, entry: FileEntry) {
    let Some(window) = anchor.root().and_downcast::<gtk::Window>() else {
        return;
    };
    let dialog = gtk::Dialog::builder()
        .title("Rename")
        .transient_for(&window)
        .modal(true)
        .build();
    dialog.add_css_class("drive-confirm-dialog");
    let field = gtk::Entry::builder()
        .text(&entry.display_name)
        .hexpand(true)
        .build();
    field.add_css_class("inline-rename");
    field.set_activates_default(true);
    dialog.content_area().append(&field);
    dialog.add_button("Cancel", gtk::ResponseType::Cancel);
    dialog
        .add_button("Rename", gtk::ResponseType::Accept)
        .add_css_class("suggested-action");
    dialog.set_default_response(gtk::ResponseType::Accept);
    let pending = Rc::new(Cell::new(false));
    let pending_operation = Rc::downgrade(&pending);
    let operation_dialog = dialog.downgrade();
    let operation_field = field.downgrade();
    state.browser.observe(move |event| {
        let Some(pending) = pending_operation.upgrade().filter(|pending| pending.get()) else {
            return;
        };
        let Some(dialog) = operation_dialog.upgrade() else {
            return;
        };
        match event {
            BrowserEvent::RenameCompleted => {
                pending.set(false);
                dialog.close();
            }
            BrowserEvent::RenameFailed { message } => {
                pending.set(false);
                dialog.set_response_sensitive(gtk::ResponseType::Accept, true);
                if let Some(field) = operation_field.upgrade() {
                    field.set_sensitive(true);
                    field.add_css_class("error");
                    field.set_tooltip_text(Some(&message));
                    field.grab_focus();
                }
            }
            _ => {}
        }
    });
    let weak = Rc::downgrade(state);
    let input = field.downgrade();
    dialog.connect_response(move |dialog, response| {
        if response == gtk::ResponseType::Accept {
            let Some(field) = input.upgrade() else { return };
            let name = field.text().to_string();
            if let Err(message) = validate_basename(&name) {
                field.add_css_class("error");
                field.set_tooltip_text(Some(message));
                field.grab_focus();
                return;
            }
            if name != entry.display_name
                && let Some(state) = weak.upgrade()
            {
                if pending.replace(true) {
                    return;
                }
                field.set_sensitive(false);
                dialog.set_response_sensitive(gtk::ResponseType::Accept, false);
                state.browser.rename(entry.clone(), name);
                return;
            }
        }
        dialog.close();
    });
    field.connect_changed(|field| {
        field.remove_css_class("error");
        field.set_tooltip_text(None);
    });
    dialog.present();
    field.grab_focus();
    field.select_region(0, rename_stem_end(&field.text()));
}

#[cfg(test)]
mod tests;
