#![expect(
    deprecated,
    reason = "An in-process GTK chooser avoids recursively calling the FileChooser portal"
)]

use std::{
    cell::{Cell, RefCell},
    collections::BTreeSet,
    ffi::OsString,
    path::Path,
    rc::Rc,
    time::{Duration, Instant},
};

use gtk::{gio, glib, prelude::*};

use crate::portal::request::{Kind, Options, Request};

mod icons;
mod visuals;

pub(crate) fn prepare() -> Rc<super::theme::ThemeManager> {
    super::window::load_styles();
    super::theme::ThemeManager::shared()
}

#[derive(Clone)]
pub(crate) struct Picker(Rc<State>);

struct State {
    dialog: gtk::FileChooserDialog,
    error: gtk::Label,
    request: Request,
    filters: Vec<(gtk::FileFilter, crate::portal::request::Filter)>,
    closed: Cell<bool>,
    busy: Cell<bool>,
    done: Rc<dyn Fn(Option<Options>)>,
    visuals: Rc<visuals::Visuals>,
    models: RefCell<Vec<glib::WeakRef<gtk::SelectionModel>>>,
    refresh_queued: Cell<bool>,
    factories: RefCell<Vec<glib::WeakRef<gtk::SignalListItemFactory>>>,
    views: RefCell<Vec<glib::WeakRef<gtk::Widget>>>,
    file_images: RefCell<Vec<glib::WeakRef<gtk::Image>>>,
    child_models: RefCell<Vec<gio::ListModel>>,
}

impl Picker {
    pub(crate) fn new(
        application: &gtk::Application,
        request: Request,
        done: Rc<dyn Fn(Option<Options>)>,
    ) -> Self {
        let action = match request.kind {
            Kind::Open {
                directory: true, ..
            }
            | Kind::SaveFiles(_) => gtk::FileChooserAction::SelectFolder,
            Kind::Open { .. } => gtk::FileChooserAction::Open,
            Kind::Save => gtk::FileChooserAction::Save,
        };
        let dialog = gtk::FileChooserDialog::new(
            Some(request.title.as_str()),
            None::<&gtk::Window>,
            action,
            &[
                ("Cancel", gtk::ResponseType::Cancel),
                (&request.accept_label, gtk::ResponseType::Accept),
            ],
        );
        dialog.set_application(Some(application));
        dialog.set_icon_name(Some(crate::identity::ICON_NAME));
        dialog.add_css_class("hermes-picker");
        dialog.set_default_size(1080, 720);
        dialog.set_modal(false);
        dialog.set_default_response(gtk::ResponseType::Accept);
        dialog.set_select_multiple(matches!(request.kind, Kind::Open { multiple: true, .. }));
        if let Some(button) = dialog.widget_for_response(gtk::ResponseType::Accept) {
            button.add_css_class("picker-accept");
        }
        let mut filters = Vec::new();
        for filter in request.filters.iter().chain(request.current_filter.iter()) {
            if filters.iter().any(|(_, existing)| existing == filter) {
                continue;
            }
            let native = gtk::FileFilter::from_gvariant(&filter.to_variant());
            dialog.add_filter(&native);
            if request.current_filter.as_ref() == Some(filter) {
                dialog.set_filter(&native);
            }
            filters.push((native, filter.clone()));
        }
        for (id, label, values, selected) in &request.choices {
            let values = values
                .iter()
                .map(|(id, label)| (id.as_str(), label.as_str()))
                .collect::<Vec<_>>();
            dialog.add_choice(id.as_str(), label.as_str(), &values);
            let selected = if selected.is_empty() {
                values.first().map_or("false", |(id, _)| *id)
            } else {
                selected.as_str()
            };
            dialog.set_choice(id, selected);
        }
        if matches!(request.kind, Kind::Save) && request.file.is_some() {
            if let Some(file) = request.file.as_ref() {
                let _file = dialog.set_file(&gio::File::for_path(file));
            }
        } else {
            let folder = request.folder.clone().unwrap_or_else(glib::home_dir);
            let _folder = dialog.set_current_folder(Some(&gio::File::for_path(folder)));
            if matches!(request.kind, Kind::Save) {
                dialog.set_current_name(request.name.as_deref().unwrap_or("Untitled"));
            }
        }
        if let Some(token) = request.activation_token.as_ref() {
            dialog.set_startup_id(token);
        }
        let error = gtk::Label::new(None);
        error.add_css_class("picker-error");
        error.set_wrap(true);
        error.set_xalign(0.0);
        error.set_visible(false);
        dialog.content_area().append(&error);
        let visuals = visuals::Visuals::new();
        attach_visuals(dialog.upcast_ref(), &visuals.widget);
        let state = Rc::new(State {
            dialog,
            error,
            request,
            filters,
            closed: Cell::new(false),
            busy: Cell::new(false),
            done,
            visuals,
            models: RefCell::new(Vec::new()),
            refresh_queued: Cell::new(false),
            factories: RefCell::new(Vec::new()),
            views: RefCell::new(Vec::new()),
            file_images: RefCell::new(Vec::new()),
            child_models: RefCell::new(Vec::new()),
        });
        watch_views(state.dialog.upcast_ref(), &state);
        let weak = Rc::downgrade(&state);
        state.dialog.connect_filter_notify(move |_| {
            if let Some(state) = weak.upgrade() {
                queue_visuals(&state);
            }
        });
        let events = gtk::EventControllerLegacy::new();
        let weak = Rc::downgrade(&state);
        events.connect_event(move |_, event| {
            if matches!(
                event.event_type(),
                gtk::gdk::EventType::ButtonRelease | gtk::gdk::EventType::KeyRelease
            ) && let Some(state) = weak.upgrade()
            {
                queue_visuals(&state);
            }
            glib::Propagation::Proceed
        });
        state.dialog.add_controller(events);
        let weak = Rc::downgrade(&state);
        state.dialog.connect_response(move |_, response| {
            let Some(state) = weak.upgrade() else {
                return;
            };
            if state.closed.get() {
                return;
            }
            if response != gtk::ResponseType::Accept {
                state.complete(None);
                return;
            }
            if state.busy.replace(true) {
                return;
            }
            state
                .dialog
                .set_response_sensitive(gtk::ResponseType::Accept, false);
            let files = state.dialog.files();
            if files.n_items() > 256 {
                state.failed("Too many selected files");
                return;
            }
            let files = (0..files.n_items())
                .filter_map(|index| files.item(index).and_downcast::<gio::File>())
                .collect::<Vec<_>>();
            glib::MainContext::default().spawn_local(async move {
                let result = selected_paths(&state.request.kind, files).await;
                if state.closed.get() {
                    return;
                }
                match result {
                    Ok(uris) => {
                        let mut results = Options::new();
                        results.insert("uris".into(), uris.to_variant());
                        let choices = state
                            .request
                            .choices
                            .iter()
                            .map(|(id, _, _, _)| {
                                (
                                    id.clone(),
                                    state
                                        .dialog
                                        .choice(id)
                                        .map(|value| value.to_string())
                                        .unwrap_or_default(),
                                )
                            })
                            .collect::<Vec<_>>();
                        results.insert("choices".into(), choices.to_variant());
                        if let Some(selected) = state.dialog.filter()
                            && let Some((_, filter)) =
                                state.filters.iter().find(|(native, _)| *native == selected)
                        {
                            results.insert("current_filter".into(), filter.to_variant());
                        }
                        results.insert(
                            "writable".into(),
                            matches!(state.request.kind, Kind::Save | Kind::SaveFiles(_))
                                .to_variant(),
                        );
                        state.complete(Some(results));
                    }
                    Err(message) => state.failed(&message),
                }
            });
        });
        let weak = Rc::downgrade(&state);
        state.dialog.connect_close_request(move |_| {
            if let Some(state) = weak.upgrade() {
                state.complete(None);
            }
            glib::Propagation::Stop
        });
        gtk::prelude::WidgetExt::realize(&state.dialog);
        let parented = attach_parent(&state.dialog, &state.request.parent);
        state.dialog.set_modal(state.request.modal && parented);
        state.dialog.present();
        queue_visuals(&state);
        Self(state)
    }

    pub(crate) fn close(&self) {
        self.0.closed.set(true);
        self.0.visuals.cancel_work();
        self.0.child_models.borrow_mut().clear();
        for image in self
            .0
            .file_images
            .borrow_mut()
            .drain(..)
            .filter_map(|image| image.upgrade())
        {
            super::thumbnail::cancel_thumbnail(&image);
        }
        let mut related = vec![self.0.dialog.clone().upcast::<gtk::Window>()];
        let windows = gtk::Window::list_toplevels()
            .into_iter()
            .filter_map(|widget| widget.downcast::<gtk::Window>().ok())
            .collect::<Vec<_>>();
        loop {
            let previous = related.len();
            for window in &windows {
                if !related.contains(window)
                    && window
                        .transient_for()
                        .is_some_and(|parent| related.contains(&parent))
                {
                    related.push(window.clone());
                }
            }
            if related.len() == previous {
                break;
            }
        }
        for window in related.into_iter().rev() {
            window.destroy();
        }
    }
}

fn attach_visuals(widget: &gtk::Widget, visuals: &gtk::Box) -> bool {
    if widget.is::<gtk::FileChooserWidget>()
        && let Some(parent) = widget.parent().and_downcast::<gtk::Box>()
    {
        let previous = widget.prev_sibling();
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        row.set_vexpand(true);
        parent.remove(widget);
        widget.set_hexpand(true);
        row.append(widget);
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .child(visuals)
            .build();
        scroller.add_css_class("picker-preview-scroll");
        visuals
            .bind_property("visible", &scroller, "visible")
            .sync_create()
            .build();
        row.append(&scroller);
        parent.insert_child_after(&row, previous.as_ref());
        return true;
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if attach_visuals(&widget, visuals) {
            return true;
        }
    }
    false
}

fn queue_visuals(state: &Rc<State>) {
    if state.closed.get() || state.refresh_queued.replace(true) {
        return;
    }
    let weak = Rc::downgrade(state);
    glib::idle_add_local_once(move || {
        let Some(state) = weak.upgrade() else {
            return;
        };
        state.refresh_queued.set(false);
        if !state.closed.get() {
            watch_views(state.dialog.upcast_ref(), &state);
            state
                .visuals
                .update(state.dialog.file(), state.dialog.current_folder());
        }
    });
}

fn watch_model(model: Option<gtk::SelectionModel>, state: &Rc<State>) {
    let Some(model) = model else {
        return;
    };
    let mut models = state.models.borrow_mut();
    models.retain(|model| model.upgrade().is_some());
    if models
        .iter()
        .any(|watched| watched.upgrade().as_ref() == Some(&model))
    {
        return;
    }
    models.push(model.downgrade());
    drop(models);
    let weak = Rc::downgrade(state);
    model.connect_selection_changed(move |_, _, _| {
        if let Some(state) = weak.upgrade() {
            queue_visuals(&state);
        }
    });
    let weak = Rc::downgrade(state);
    model.connect_items_changed(move |_, _, _, _| {
        if let Some(state) = weak.upgrade() {
            queue_visuals(&state);
        }
    });
    queue_visuals(state);
}

fn watch_views(widget: &gtk::Widget, state: &Rc<State>) {
    icons::decorate(widget, state);
    if widget.is::<gtk::DropDown>() {
        widget.add_css_class("picker-file-type");
        widget.set_tooltip_text(Some("File type"));
    }
    let mut views = state.views.borrow_mut();
    views.retain(|view| view.upgrade().is_some());
    let watched = views
        .iter()
        .any(|view| view.upgrade().as_ref() == Some(widget));
    if !watched
        && (widget.is::<gtk::ColumnView>()
            || widget.is::<gtk::GridView>()
            || widget.is::<gtk::ScrolledWindow>()
            || widget.is::<gtk::Stack>())
    {
        views.push(widget.downgrade());
    }
    drop(views);
    if !watched {
        if let Some(scroller) = widget.downcast_ref::<gtk::ScrolledWindow>() {
            let weak = Rc::downgrade(state);
            scroller.connect_child_notify(move |_| {
                if let Some(state) = weak.upgrade() {
                    queue_visuals(&state);
                }
            });
        } else if let Some(stack) = widget.downcast_ref::<gtk::Stack>() {
            let weak = Rc::downgrade(state);
            stack.connect_visible_child_notify(move |_| {
                if let Some(state) = weak.upgrade() {
                    queue_visuals(&state);
                }
            });
        }
    }
    if let Some(view) = widget.downcast_ref::<gtk::ColumnView>() {
        watch_model(view.model(), state);
        for position in 0..view.columns().n_items() {
            if let Some(column) = view
                .columns()
                .item(position)
                .and_downcast::<gtk::ColumnViewColumn>()
            {
                icons::watch_factory(column.factory(), state);
            }
        }
        if !watched {
            let weak = Rc::downgrade(state);
            view.connect_model_notify(move |view| {
                if let Some(state) = weak.upgrade() {
                    watch_model(view.model(), &state);
                }
            });
        }
    } else if let Some(view) = widget.downcast_ref::<gtk::GridView>() {
        view.add_css_class("file-grid");
        watch_model(view.model(), state);
        icons::watch_factory(view.factory(), state);
        if !watched {
            let children = view.observe_children();
            let weak = Rc::downgrade(state);
            children.connect_items_changed(move |_, _, _, _| {
                if let Some(state) = weak.upgrade() {
                    queue_visuals(&state);
                }
            });
            state.child_models.borrow_mut().push(children);
            let weak = Rc::downgrade(state);
            view.connect_model_notify(move |view| {
                if let Some(state) = weak.upgrade() {
                    watch_model(view.model(), &state);
                }
            });
        }
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        watch_views(&widget, state);
    }
}

fn attach_parent(dialog: &gtk::FileChooserDialog, parent: &str) -> bool {
    if parent.is_empty() {
        return false;
    }
    #[cfg(target_os = "linux")]
    if let Some(handle) = parent.strip_prefix("wayland:")
        && let Some(surface) = dialog
            .surface()
            .and_downcast::<gdk4_wayland::WaylandToplevel>()
    {
        return surface.set_transient_for_exported(handle);
    }
    tracing::debug!("Chooser parent is unavailable on this display; keeping a non-modal chooser");
    false
}

impl State {
    fn failed(&self, message: &str) {
        self.error.set_text(message);
        self.error.set_visible(true);
        self.busy.set(false);
        self.dialog
            .set_response_sensitive(gtk::ResponseType::Accept, true);
    }

    fn complete(&self, results: Option<Options>) {
        if self.closed.replace(true) {
            return;
        }
        (self.done)(results);
    }
}

async fn info(
    file: &gio::File,
    deadline: Instant,
    follow: bool,
) -> Result<Option<gio::FileInfo>, String> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err("The selected location is taking too long to respond".into());
    }
    let flags = if follow {
        gio::FileQueryInfoFlags::NONE
    } else {
        gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS
    };
    match glib::future_with_timeout(
        remaining,
        file.query_info_future("standard::type", flags, glib::Priority::DEFAULT),
    )
    .await
    .map_err(|_| "The selected location is taking too long to respond")?
    {
        Ok(info) => Ok(Some(info)),
        Err(error) if error.matches(gio::IOErrorEnum::NotFound) => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

fn numbered(name: &std::ffi::OsStr, number: usize) -> OsString {
    if number == 0 {
        return name.to_owned();
    }
    let path = Path::new(name);
    let mut candidate = path.file_stem().unwrap_or(name).to_owned();
    candidate.push(format!(" ({number})"));
    if let Some(extension) = path.extension() {
        candidate.push(".");
        candidate.push(extension);
    }
    candidate
}

async fn selected_paths(kind: &Kind, files: Vec<gio::File>) -> Result<Vec<String>, String> {
    if files.is_empty() {
        return Err("Choose a file or folder first".into());
    }
    if files.len() > 256 {
        return Err("Too many selected files".into());
    }
    if !matches!(kind, Kind::Open { multiple: true, .. }) && files.len() != 1 {
        return Err("Choose one destination".into());
    }
    let files = files.into_iter().map(|file| {
        file.path().filter(|path| path.is_absolute()).map(gio::File::for_path)
            .ok_or_else(|| "Choose a local or mounted folder; this picker returns local file locations only".to_owned())
    }).collect::<Result<Vec<_>, _>>()?;
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut uris = Vec::new();
    match kind {
        Kind::SaveFiles(names) => {
            let folder = &files[0];
            if !info(folder, deadline, true)
                .await?
                .is_some_and(|info| info.file_type() == gio::FileType::Directory)
            {
                return Err("Choose an existing destination folder".into());
            }
            let mut reserved = BTreeSet::new();
            let mut attempts = 0;
            for name in names {
                let mut selected = None;
                for number in 0..256 {
                    attempts += 1;
                    if attempts > 256 {
                        return Err(
                            "Too many name conflicts; choose another destination folder".into()
                        );
                    }
                    let candidate = numbered(name, number);
                    if !crate::portal::request::filename(candidate.as_encoded_bytes()) {
                        return Err("A generated output name is too long".into());
                    }
                    if reserved.contains(&candidate) {
                        continue;
                    }
                    let file = folder.child(&candidate);
                    if info(&file, deadline, false).await?.is_none() {
                        reserved.insert(candidate);
                        selected = Some(file.uri().to_string());
                        break;
                    }
                }
                uris.push(selected.ok_or("Unable to choose a non-conflicting output name")?);
            }
        }
        _ => {
            for file in files {
                let metadata = info(&file, deadline, true).await?;
                match kind {
                    Kind::Open {
                        directory: true, ..
                    } if !metadata
                        .as_ref()
                        .is_some_and(|info| info.file_type() == gio::FileType::Directory) =>
                    {
                        return Err("Choose an existing folder".into());
                    }
                    Kind::Open {
                        directory: false, ..
                    } if metadata
                        .as_ref()
                        .is_none_or(|info| info.file_type() == gio::FileType::Directory) =>
                    {
                        return Err("Choose an existing file".into());
                    }
                    Kind::Save
                        if metadata
                            .as_ref()
                            .is_some_and(|info| info.file_type() == gio::FileType::Directory) =>
                    {
                        return Err("Choose a filename, not an existing folder".into());
                    }
                    _ => {}
                }
                uris.push(file.uri().to_string());
            }
        }
    }
    Ok(uris)
}

#[cfg(test)]
mod tests;
