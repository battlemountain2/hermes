use super::*;

pub(super) fn install(state: &Rc<State>) {
    let button = gtk::MenuButton::new();
    button.add_css_class("picker-new-folder");
    button.set_tooltip_text(Some("Create a folder here (Ctrl+Shift+N)"));
    let caption = gtk::Box::new(gtk::Orientation::Horizontal, 7);
    caption.append(&crate::assets::primary_icon(
        crate::assets::icons::FOLDER,
        16,
    ));
    caption.append(&gtk::Label::new(Some("New folder")));
    button.set_child(Some(&caption));
    let popover = gtk::Popover::new();
    popover.add_css_class("hermes-picker-popover");
    let content = gtk::Box::new(gtk::Orientation::Vertical, 10);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);
    let title = gtk::Label::new(Some("New folder"));
    title.set_xalign(0.0);
    content.append(&title);
    let name = gtk::Entry::new();
    name.set_placeholder_text(Some("Folder name"));
    name.set_activates_default(false);
    name.set_width_chars(24);
    content.append(&name);
    let error = gtk::Label::new(None);
    error.set_wrap(true);
    error.set_max_width_chars(30);
    error.set_xalign(0.0);
    error.add_css_class("picker-error");
    error.set_visible(false);
    content.append(&error);
    let create = gtk::Button::with_label("Create folder");
    create.add_css_class("picker-accept");
    create.set_sensitive(false);
    content.append(&create);
    popover.set_child(Some(&content));
    button.set_popover(Some(&popover));
    let name_for_open = name.downgrade();
    button.connect_notify_local(Some("active"), move |button, _| {
        if button.is_active()
            && let Some(name) = name_for_open.upgrade()
        {
            name.grab_focus();
        }
    });
    if let Some(header) = state.dialog.titlebar().and_downcast::<gtk::HeaderBar>() {
        header.pack_start(&button);
    } else {
        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        toolbar.add_css_class("picker-folder-toolbar");
        toolbar.set_halign(gtk::Align::End);
        toolbar.append(&button);
        state.dialog.content_area().prepend(&toolbar);
    }
    let changed_create = create.downgrade();
    let changed_error = error.downgrade();
    name.connect_changed(move |name| {
        if let Some(create) = changed_create.upgrade() {
            create.set_sensitive(crate::services::validate_basename(&name.text()).is_ok());
        }
        if let Some(error) = changed_error.upgrade() {
            error.set_visible(false);
        }
    });
    let weak = Rc::downgrade(state);
    let create_name = name.clone();
    let create_popover = popover.downgrade();
    let create_error = error.clone();
    create.connect_clicked(move |create| {
        let Some(state) = weak
            .upgrade()
            .filter(|state| !state.closed.get() && !state.busy.get())
        else {
            return;
        };
        let Some(parent) = state
            .dialog
            .current_folder()
            .filter(|file| file.path().is_some())
        else {
            create_error.set_text("Choose a local folder first.");
            create_error.set_visible(true);
            return;
        };
        let folder_name = create_name.text().to_string();
        if let Err(message) = crate::services::validate_basename(&folder_name) {
            create_error.set_text(message);
            create_error.set_visible(true);
            return;
        }
        state.busy.set(true);
        state
            .dialog
            .set_response_sensitive(gtk::ResponseType::Accept, false);
        create.set_sensitive(false);
        create_name.set_sensitive(false);
        let name = create_name.clone();
        let popover = create_popover.clone();
        let error = create_error.clone();
        let create = create.clone();
        let weak = Rc::downgrade(&state);
        let saved_name = if matches!(state.request.kind, Kind::Save) {
            state.dialog.current_name()
        } else {
            None
        };
        glib::MainContext::default().spawn_local(async move {
            let result = create_folder(parent, &folder_name).await;
            let Some(state) = weak.upgrade().filter(|state| !state.closed.get()) else {
                return;
            };
            state.busy.set(false);
            state
                .dialog
                .set_response_sensitive(gtk::ResponseType::Accept, true);
            create.set_sensitive(true);
            name.set_sensitive(true);
            match result {
                Ok(folder) => match state.dialog.set_current_folder(Some(&folder)) {
                    Ok(true) => {
                        if let Some(saved) = saved_name {
                            state.dialog.set_current_name(&saved);
                        }
                        name.set_text("");
                        if let Some(popover) = popover.upgrade() {
                            popover.popdown();
                        }
                        queue_visuals(&state);
                    }
                    Err(_) | Ok(false) => {
                        error.set_text("Folder created. Open it from the file list to continue.");
                        error.set_visible(true);
                    }
                },
                Err(message) => {
                    error.set_text(&message);
                    error.set_visible(true);
                }
            }
        });
    });
    let enter_create = create.clone();
    name.connect_activate(move |_| {
        if enter_create.is_sensitive() {
            enter_create.emit_clicked();
        }
    });
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = Rc::downgrade(state);
    let shortcut_button = button.downgrade();
    keys.connect_key_pressed(move |_, key, _, modifiers| {
        if modifiers
            .contains(gtk::gdk::ModifierType::CONTROL_MASK | gtk::gdk::ModifierType::SHIFT_MASK)
            && !modifiers.contains(gtk::gdk::ModifierType::ALT_MASK)
            && matches!(key, gtk::gdk::Key::n | gtk::gdk::Key::N)
            && weak
                .upgrade()
                .is_some_and(|state| !state.busy.get() && !state.closed.get())
        {
            if let Some(button) = shortcut_button.upgrade() {
                button.popup();
            }
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    state.dialog.add_controller(keys);
}

async fn create_folder(parent: gio::File, name: &str) -> Result<gio::File, String> {
    crate::services::validate_basename(name).map_err(str::to_owned)?;
    if parent.path().is_none() {
        return Err("Choose a local folder first.".into());
    }
    let folder = parent.child(name);
    folder
        .make_directory_future(glib::Priority::DEFAULT)
        .await
        .map_err(|error| {
            if error.matches(gio::IOErrorEnum::Exists) {
                "An item with that name already exists.".to_owned()
            } else {
                format!("Could not create the folder: {error}")
            }
        })?;
    Ok(folder)
}

#[cfg(test)]
mod tests;
