use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    time::Duration,
};

use gtk::{gdk, glib, prelude::*};

use crate::{
    assets::{self, icons},
    services::{
        ScopedSearch, SearchEntityKey, SearchGroup, SearchScope,
        saved_searches::{Command, SavedSearch, Snapshot, Store},
    },
};

#[cfg(test)]
mod tests;

const WIDTH: i32 = 800;
const HEIGHT: i32 = 600;
const POLL_INTERVAL: Duration = Duration::from_millis(100);
const GROUPS: [SearchGroup; 3] = [SearchGroup::All, SearchGroup::Files, SearchGroup::Songs];

#[derive(Clone)]
pub struct SavedSearchDialog {
    state: Rc<State>,
}

struct State {
    layer: gtk::Box,
    list: gtk::ListBox,
    list_status: gtk::Label,
    list_stack: gtk::Stack,
    editor: gtk::Box,
    editor_title: gtk::Label,
    name: gtk::Entry,
    query: gtk::Entry,
    scope: gtk::DropDown,
    root: gtk::Entry,
    group: gtk::DropDown,
    membership: gtk::Label,
    status: gtk::Label,
    confirmation: gtk::Stack,
    confirmation_text: gtk::Label,
    confirm: gtk::Button,
    cancel: gtk::Button,
    save: gtk::Button,
    delete: gtk::Button,
    open: gtk::Button,
    create: gtk::Button,
    load: gtk::Button,
    reload: gtk::Button,
    store: Store,
    entries: RefCell<Vec<SavedSearch>>,
    previous: RefCell<Option<SavedSearch>>,
    context: RefCell<Option<ScopedSearch>>,
    pending: RefCell<Option<Pending>>,
    question: Cell<Option<Question>>,
    ready: Cell<bool>,
    writable: Cell<bool>,
    revision: Cell<Option<u64>>,
    store_error: RefCell<Option<String>>,
    rendering: Cell<bool>,
    poll_source: RefCell<Option<glib::SourceId>>,
    open_callback: Rc<dyn Fn(String)>,
    dismiss: Rc<dyn Fn()>,
}

#[derive(Clone, Copy)]
enum Question {
    Delete,
    ClearMembership(u32),
}

struct Pending {
    revision: u64,
    operation: Operation,
}

enum Operation {
    Create {
        name: String,
        uri: String,
        existing_ids: Vec<u64>,
    },
    Update {
        previous: SavedSearch,
        name: String,
        uri: String,
    },
    Delete(SavedSearch),
    Reload,
}

impl Drop for State {
    fn drop(&mut self) {
        if let Some(source) = self.poll_source.get_mut().take() {
            source.remove();
        }
    }
}

impl SavedSearchDialog {
    pub fn new(store: Store, open_callback: Rc<dyn Fn(String)>, dismiss: Rc<dyn Fn()>) -> Self {
        let layer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        layer.add_css_class("search-backdrop");
        layer.add_css_class("app-modal-layer");
        layer.add_css_class("saved-searches");
        layer.set_hexpand(true);
        layer.set_vexpand(true);
        layer.set_focusable(true);
        layer.set_visible(false);

        let panel = gtk::Box::new(gtk::Orientation::Vertical, 0);
        panel.add_css_class("settings-page");
        let titlebar = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        titlebar.add_css_class("settings-titlebar");
        titlebar.append(&assets::primary_icon(icons::SEARCH, 18));
        let title = label("Saved Searches / Smart Folders", "settings-title");
        title.set_hexpand(true);
        titlebar.append(&title);
        let close = gtk::Button::new();
        close.add_css_class("settings-close");
        close.set_child(Some(&assets::primary_icon(icons::X, 18)));
        close.set_tooltip_text(Some("Close saved searches (Esc)"));
        close.update_property(&[gtk::accessible::Property::Label("Close saved searches")]);
        titlebar.append(&close);
        panel.append(&titlebar);

        let status = label("Loading saved searches…", "search-notice");
        status.set_accessible_role(gtk::AccessibleRole::Status);
        status.set_single_line_mode(true);
        status.set_tooltip_text(Some("Loading saved searches…"));
        status.set_size_request(-1, 36);
        panel.append(&status);

        let body = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        body.set_vexpand(true);
        let navigation = gtk::Box::new(gtk::Orientation::Vertical, 8);
        navigation.add_css_class("settings-navigation");
        navigation.append(&label("SAVED SEARCHES", "search-filter-title"));
        let create = button("New saved search", false);
        navigation.append(&create);
        let list = gtk::ListBox::new();
        list.add_css_class("search-results");
        list.set_selection_mode(gtk::SelectionMode::Single);
        list.set_activate_on_single_click(false);
        list.update_property(&[gtk::accessible::Property::Label("Saved searches")]);
        let list_scroll = scroller(&list);
        let list_status = label("Loading…", "settings-option-description");
        list_status.set_wrap(true);
        list_status.set_max_width_chars(22);
        let list_stack = gtk::Stack::new();
        list_stack.set_transition_type(gtk::StackTransitionType::None);
        list_stack.set_transition_duration(0);
        list_stack.set_vexpand(true);
        list_stack.add_named(&list_scroll, Some("list"));
        list_stack.add_named(&list_status, Some("status"));
        list_stack.set_visible_child_name("status");
        navigation.append(&list_stack);
        let load = button("Load selected", false);
        load.set_tooltip_text(Some(
            "Replace editor fields with the current saved query. Unsaved edits are discarded.",
        ));
        navigation.append(&load);
        let navigation_scroll = scroller(&navigation);
        navigation_scroll.set_min_content_width(250);
        navigation_scroll.set_max_content_width(250);
        body.append(&navigation_scroll);

        let editor = gtk::Box::new(gtk::Orientation::Vertical, 12);
        editor.add_css_class("settings-preferences");
        let editor_title = label("Choose a saved search", "settings-title");
        editor.append(&editor_title);
        let name = entry("Saved search name", "Name", 80);
        append_field(&editor, "Name", &name);
        let query = entry(
            "Search query and filters",
            "Query, including any filters",
            4096,
        );
        append_field(&editor, "Query and filters", &query);
        let query_hint = label(
            "Uses the same query and filters as Search. No files are moved or changed.",
            "settings-option-description",
        );
        query_hint.set_wrap(true);
        editor.append(&query_hint);
        let scope = gtk::DropDown::from_strings(&["This folder", "Home", "Music library"]);
        scope.update_property(&[gtk::accessible::Property::Label("Search scope")]);
        append_field(&editor, "Scope", &scope);
        let root = entry("Search root folder", "/absolute/folder/path", 4096);
        append_field(&editor, "Location", &root);
        root.set_tooltip_text(Some(
            "An absolute local folder path. Home and Music library use their standard locations. This edits only the saved query.",
        ));
        let group = gtk::DropDown::from_strings(&["All", "Files", "Songs"]);
        group.update_property(&[gtk::accessible::Property::Label("Result group")]);
        append_field(&editor, "Results", &group);
        let virtual_hint = label(
            "To save an album or artist, open it in Search first, then save its Songs.",
            "settings-option-description",
        );
        virtual_hint.set_wrap(true);
        editor.append(&virtual_hint);
        let membership = label(
            "No album or artist membership filter.",
            "settings-option-description",
        );
        membership.set_wrap(true);
        editor.append(&membership);
        let editor_scroll = scroller(&editor);
        editor_scroll.set_hexpand(true);
        body.append(&editor_scroll);
        panel.append(&body);

        let confirmation = gtk::Stack::new();
        confirmation.set_transition_type(gtk::StackTransitionType::None);
        confirmation.set_transition_duration(0);
        confirmation.set_size_request(-1, 84);
        let blank = gtk::Box::new(gtk::Orientation::Vertical, 0);
        confirmation.add_named(&blank, Some("none"));
        let prompt = gtk::Box::new(gtk::Orientation::Vertical, 6);
        prompt.set_margin_start(16);
        prompt.set_margin_end(16);
        let confirmation_text = label("", "settings-option-description");
        confirmation_text.set_wrap(true);
        confirmation_text.set_lines(2);
        confirmation_text.set_ellipsize(gtk::pango::EllipsizeMode::End);
        prompt.append(&confirmation_text);
        let confirmation_actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let confirm = button("Delete saved search", true);
        let cancel = button("Cancel", false);
        confirmation_actions.append(&cancel);
        confirmation_actions.append(&confirm);
        prompt.append(&confirmation_actions);
        confirmation.add_named(&prompt, Some("confirm"));
        confirmation.set_visible_child_name("none");
        panel.append(&confirmation);

        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        footer.add_css_class("search-footer");
        footer.set_size_request(-1, 48);
        let reload = button("Reload", false);
        reload.set_tooltip_text(Some("Reload saved searches from storage (Ctrl+R)"));
        let delete = button("Delete", false);
        delete.set_tooltip_text(Some(
            "Delete only the selected saved query, never its files",
        ));
        let open = button("Open", false);
        open.set_tooltip_text(Some(
            "Open saved results in the existing window (Alt+Enter)",
        ));
        let save = button("Save", true);
        save.set_tooltip_text(Some("Save changes to storage (Ctrl+S)"));
        footer.append(&reload);
        footer.append(&delete);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        footer.append(&spacer);
        footer.append(&open);
        footer.append(&save);
        panel.append(&footer);

        let frame = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_width(false)
            .propagate_natural_height(false)
            .min_content_width(WIDTH)
            .max_content_width(WIDTH)
            .min_content_height(HEIGHT)
            .max_content_height(HEIGHT)
            .halign(gtk::Align::Center)
            .valign(gtk::Align::Center)
            .child(&panel)
            .build();
        frame.add_css_class("search-dialog");
        frame.set_size_request(WIDTH, HEIGHT);
        frame.set_overflow(gtk::Overflow::Hidden);
        let top_spacer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        top_spacer.set_vexpand(true);
        let bottom_spacer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        bottom_spacer.set_vexpand(true);
        layer.append(&top_spacer);
        layer.append(&frame);
        layer.append(&bottom_spacer);

        let state = Rc::new(State {
            layer,
            list,
            list_status,
            list_stack,
            editor,
            editor_title,
            name,
            query,
            scope,
            root,
            group,
            membership,
            status,
            confirmation,
            confirmation_text,
            confirm,
            cancel,
            save,
            delete,
            open,
            create,
            load,
            reload,
            store,
            entries: RefCell::new(Vec::new()),
            previous: RefCell::new(None),
            context: RefCell::new(None),
            pending: RefCell::new(None),
            question: Cell::new(None),
            ready: Cell::new(false),
            writable: Cell::new(false),
            revision: Cell::new(None),
            store_error: RefCell::new(None),
            rendering: Cell::new(false),
            poll_source: RefCell::new(None),
            open_callback,
            dismiss,
        });
        connect(&state, &close);
        update_controls(&state);
        Self { state }
    }

    pub fn widget(&self) -> gtk::Box {
        self.state.layer.clone()
    }

    pub fn show_create(&self, uri: &str) {
        let state = &self.state;
        state.layer.set_visible(true);
        poll(state);
        if state.pending.borrow().is_none() {
            match ScopedSearch::from_uri(uri) {
                Ok(search) => load_editor(state, None, Some(search)),
                Err(error) => {
                    load_editor(state, None, None);
                    message(state, &format!("Cannot save this search: {error}"));
                }
            }
        }
        start_polling(state);
        if !state.name.grab_focus() {
            state.layer.grab_focus();
        }
    }

    pub fn show_manage(&self) {
        let state = &self.state;
        state.layer.set_visible(true);
        poll(state);
        if state.context.borrow().is_none() && state.pending.borrow().is_none() {
            let first = state.entries.borrow().first().cloned();
            select_entry(state, first);
        }
        start_polling(state);
        if state.entries.borrow().is_empty() {
            if !state.create.grab_focus() {
                state.layer.grab_focus();
            }
        } else {
            state.list.grab_focus();
        }
    }

    #[cfg(test)]
    pub fn hide(&self) {
        hide(&self.state);
    }
}

fn label(text: &str, class: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(text));
    label.add_css_class(class);
    label.set_xalign(0.0);
    label.set_max_width_chars(1);
    label.set_hexpand(true);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label
}

fn entry(accessible_name: &str, placeholder: &str, limit: i32) -> gtk::Entry {
    let entry = gtk::Entry::builder()
        .placeholder_text(placeholder)
        .max_length(limit)
        .hexpand(true)
        .build();
    entry.add_css_class("search-field");
    entry.add_css_class("search-filter-entry");
    entry.update_property(&[gtk::accessible::Property::Label(accessible_name)]);
    entry
}

fn button(text: &str, primary: bool) -> gtk::Button {
    let button = gtk::Button::with_label(text);
    button.add_css_class(if primary {
        "search-filter-apply"
    } else {
        "search-filter-clear"
    });
    button.set_valign(gtk::Align::Center);
    button
}

fn append_field(editor: &gtk::Box, title: &str, control: &impl IsA<gtk::Widget>) {
    editor.append(&label(title, "search-filter-label"));
    editor.append(control);
}

fn scroller(child: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .propagate_natural_width(false)
        .propagate_natural_height(false)
        .vexpand(true)
        .child(child)
        .build()
}

fn connect(state: &Rc<State>, close: &gtk::Button) {
    let weak = Rc::downgrade(state);
    state.layer.connect_unmap(move |_| {
        if let Some(state) = weak.upgrade()
            && state.layer.is_visible()
        {
            hide(&state);
        }
    });
    let weak = Rc::downgrade(state);
    close.connect_clicked(move |_| {
        if let Some(state) = weak.upgrade() {
            hide(&state);
        }
    });
    let weak = Rc::downgrade(state);
    state.create.connect_clicked(move |_| {
        if let Some(state) = weak.upgrade() {
            if !can_mutate(&state) || state.question.get().is_some() {
                return;
            }
            let search = ScopedSearch::new(SearchScope::Home, &PathBuf::new(), "");
            load_editor(&state, None, Some(search));
            message(
                &state,
                "New saved search. Files are never moved or changed.",
            );
            state.name.grab_focus();
        }
    });
    let weak = Rc::downgrade(state);
    state.load.connect_clicked(move |_| {
        if let Some(state) = weak.upgrade() {
            if !state.load.is_sensitive() {
                return;
            }
            let selected = state
                .list
                .selected_row()
                .and_then(|row| usize::try_from(row.index()).ok())
                .and_then(|position| state.entries.borrow().get(position).cloned());
            select_entry(&state, selected);
            message(&state, "Loaded the selected saved query into the editor.");
            state.name.grab_focus();
        }
    });
    let weak = Rc::downgrade(state);
    state.list.connect_row_selected(move |_, row| {
        if let Some(state) = weak.upgrade() {
            if state.rendering.get()
                || state.pending.borrow().is_some()
                || state.question.get().is_some()
            {
                return;
            }
            let selected = row
                .and_then(|row| usize::try_from(row.index()).ok())
                .and_then(|position| state.entries.borrow().get(position).cloned());
            select_entry(&state, selected);
        }
    });
    let weak = Rc::downgrade(state);
    state.list.connect_row_activated(move |_, _| {
        if let Some(state) = weak.upgrade() {
            open_selected(&state);
        }
    });
    for field in [&state.name, &state.query, &state.root] {
        let weak = Rc::downgrade(state);
        field.connect_changed(move |_| {
            if let Some(state) = weak.upgrade() {
                if !state.rendering.get() {
                    update_controls(&state);
                }
            }
        });
    }
    let weak = Rc::downgrade(state);
    state.scope.connect_selected_notify(move |_| {
        if let Some(state) = weak.upgrade() {
            if state.rendering.get() {
                return;
            }
            if selected_scope(&state) != SearchScope::ThisFolder {
                let root = selected_scope(&state).root(&PathBuf::new());
                state.root.set_text(&root.to_string_lossy());
            }
            update_controls(&state);
        }
    });
    let weak = Rc::downgrade(state);
    state.group.connect_selected_notify(move |_| {
        if let Some(state) = weak.upgrade() {
            if state.rendering.get() {
                return;
            }
            let selected = state.group.selected();
            let has_membership = state
                .context
                .borrow()
                .as_ref()
                .is_some_and(|search| search.entity.is_some());
            if selected != 2 && has_membership {
                state.rendering.set(true);
                state.group.set_selected(2);
                state.rendering.set(false);
                ask(&state, Question::ClearMembership(selected));
            }
            update_controls(&state);
        }
    });
    let weak = Rc::downgrade(state);
    state.save.connect_clicked(move |_| {
        if let Some(state) = weak.upgrade() {
            save(&state);
        }
    });
    let weak = Rc::downgrade(state);
    state.delete.connect_clicked(move |_| {
        if let Some(state) = weak.upgrade() {
            if state.delete.is_sensitive() {
                ask(&state, Question::Delete);
            }
        }
    });
    let weak = Rc::downgrade(state);
    state.confirm.connect_clicked(move |_| {
        if let Some(state) = weak.upgrade() {
            confirm(&state);
        }
    });
    let weak = Rc::downgrade(state);
    state.cancel.connect_clicked(move |_| {
        if let Some(state) = weak.upgrade() {
            cancel_question(&state);
        }
    });
    let weak = Rc::downgrade(state);
    state.open.connect_clicked(move |_| {
        if let Some(state) = weak.upgrade() {
            open_selected(&state);
        }
    });
    let weak = Rc::downgrade(state);
    state.reload.connect_clicked(move |_| {
        if let Some(state) = weak.upgrade() {
            reload(&state);
        }
    });
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak = Rc::downgrade(state);
    keys.connect_key_pressed(move |_, key, _, modifiers| {
        let Some(state) = weak.upgrade() else {
            return glib::Propagation::Proceed;
        };
        if key == gdk::Key::Escape {
            hide(&state);
        } else if matches!(key, gdk::Key::Tab | gdk::Key::ISO_Left_Tab) {
            let direction = if key == gdk::Key::ISO_Left_Tab
                || modifiers.contains(gdk::ModifierType::SHIFT_MASK)
            {
                gtk::DirectionType::TabBackward
            } else {
                gtk::DirectionType::TabForward
            };
            if !state.layer.child_focus(direction) {
                state.layer.set_focus_child(None::<&gtk::Widget>);
                state.layer.child_focus(direction);
            }
        } else if modifiers.contains(gdk::ModifierType::CONTROL_MASK)
            && matches!(key, gdk::Key::s | gdk::Key::S)
        {
            save(&state);
        } else if modifiers.contains(gdk::ModifierType::CONTROL_MASK)
            && matches!(key, gdk::Key::r | gdk::Key::R)
        {
            reload(&state);
        } else if modifiers.contains(gdk::ModifierType::ALT_MASK) && key == gdk::Key::Return {
            open_selected(&state);
        } else {
            return glib::Propagation::Proceed;
        }
        glib::Propagation::Stop
    });
    state.layer.add_controller(keys);
}

fn hide(state: &State) {
    state.layer.set_visible(false);
    if let Some(source) = state.poll_source.borrow_mut().take() {
        source.remove();
    }
    state.question.set(None);
    state.confirmation.set_visible_child_name("none");
    update_controls(state);
    (state.dismiss)();
}

fn start_polling(state: &Rc<State>) {
    if state.poll_source.borrow().is_some() {
        return;
    }
    let weak = Rc::downgrade(state);
    let source = glib::timeout_add_local(POLL_INTERVAL, move || {
        let Some(state) = weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        if !state.layer.is_visible() {
            state.poll_source.borrow_mut().take();
            return glib::ControlFlow::Break;
        }
        poll(&state);
        glib::ControlFlow::Continue
    });
    state.poll_source.replace(Some(source));
}

fn poll(state: &State) {
    if !state.layer.is_visible() || state.revision.get() == Some(state.store.revision()) {
        return;
    }
    apply_snapshot(state, state.store.snapshot());
}

fn apply_snapshot(state: &State, snapshot: Snapshot) {
    state.revision.set(Some(snapshot.revision));
    state.ready.set(snapshot.ready);
    state.writable.set(snapshot.writable);
    state.store_error.replace(snapshot.error.clone());
    let outcome = state.pending.borrow().as_ref().and_then(|pending| {
        if snapshot.revision <= pending.revision {
            return None;
        }
        if let Some(error) = &snapshot.error {
            return Some(Err(error.clone()));
        }
        if !snapshot.ready {
            return None;
        }
        match &pending.operation {
            Operation::Create {
                name,
                uri,
                existing_ids,
            } => snapshot
                .entries
                .iter()
                .find(|entry| {
                    !existing_ids.contains(&entry.id) && entry.name == *name && entry.uri == *uri
                })
                .cloned()
                .map(|entry| Ok(Some(entry))),
            Operation::Update {
                previous,
                name,
                uri,
            } => snapshot
                .entries
                .iter()
                .find(|entry| entry.id == previous.id && entry.name == *name && entry.uri == *uri)
                .cloned()
                .map(|entry| Ok(Some(entry))),
            Operation::Delete(previous) => {
                (!snapshot.entries.iter().any(|entry| entry.id == previous.id)).then_some(Ok(None))
            }
            Operation::Reload => Some(Ok(None)),
        }
    });
    state.entries.replace(snapshot.entries);
    render_list(state);
    if let Some(outcome) = outcome {
        let pending = state.pending.borrow_mut().take();
        match outcome {
            Err(error) => message(
                state,
                &format!("Not saved: {error}. Your editor fields are unchanged."),
            ),
            Ok(selected) => {
                let operation = pending.map(|pending| pending.operation);
                match operation {
                    Some(Operation::Create { .. } | Operation::Update { .. }) => {
                        select_entry(state, selected);
                        message(state, "Saved to storage. Files are unchanged.");
                    }
                    Some(Operation::Delete(_)) => {
                        let next = state.entries.borrow().first().cloned();
                        select_entry(state, next);
                        message(state, "Saved search deleted. No files were deleted.");
                    }
                    Some(Operation::Reload) => message(
                        state,
                        "Saved searches reloaded. Editor fields are unchanged; use Load selected to load the current version.",
                    ),
                    None => {}
                }
            }
        }
    } else if state.pending.borrow().is_none() {
        if let Some(error) = &snapshot.error {
            message(
                state,
                &format!(
                    "{}: {error}",
                    if snapshot.writable {
                        "Saved search was not changed"
                    } else {
                        "Saved-search storage unavailable"
                    }
                ),
            );
        } else if !snapshot.ready {
            message(state, "Loading saved searches…");
        } else if state
            .previous
            .borrow()
            .as_ref()
            .is_some_and(|previous| !state.entries.borrow().contains(previous))
        {
            message(
                state,
                "This saved search changed elsewhere. Your fields are kept; saving uses the original version and may be rejected. Use Load selected to load the current version.",
            );
        } else {
            message(
                state,
                "Saved queries only. Files are never moved or changed.",
            );
        }
    }
    update_controls(state);
}

fn render_list(state: &State) {
    state.rendering.set(true);
    while let Some(child) = state.list.first_child() {
        state.list.remove(&child);
    }
    let previous = state.previous.borrow();
    for entry in state.entries.borrow().iter() {
        let row = gtk::ListBoxRow::new();
        row.add_css_class("search-result");
        let contents = gtk::Box::new(gtk::Orientation::Vertical, 4);
        contents.append(&label(&entry.name, "search-result-name"));
        let summary = ScopedSearch::from_uri(&entry.uri)
            .map(|search| {
                if search.query.is_empty() {
                    group_name(search.group).to_owned()
                } else {
                    search.query
                }
            })
            .unwrap_or_else(|_| "Invalid saved query".into());
        contents.append(&label(&summary, "search-result-path"));
        row.set_tooltip_text(Some(&format!("{}\n{summary}", entry.name)));
        row.set_child(Some(&contents));
        state.list.append(&row);
        if previous
            .as_ref()
            .is_some_and(|previous| previous.id == entry.id)
        {
            state.list.select_row(Some(&row));
        }
    }
    if state.entries.borrow().is_empty() {
        state.list_status.set_text(if !state.ready.get() {
            "Loading saved searches…"
        } else if !state.writable.get() {
            "Storage unavailable. Use Reload to try again."
        } else {
            "No saved searches yet. Save a query from Search, or choose New saved search."
        });
        state.list_stack.set_visible_child_name("status");
    } else {
        state.list_stack.set_visible_child_name("list");
    }
    state.rendering.set(false);
}

fn select_entry(state: &State, selected: Option<SavedSearch>) {
    let search = selected
        .as_ref()
        .map(|entry| ScopedSearch::from_uri(&entry.uri));
    match search {
        Some(Ok(search)) => load_editor(state, selected, Some(search)),
        Some(Err(error)) => {
            load_editor(state, selected, None);
            message(
                state,
                &format!("Cannot edit this saved search: {error}. It can still be deleted."),
            );
        }
        None => load_editor(state, None, None),
    }
}

fn load_editor(state: &State, previous: Option<SavedSearch>, search: Option<ScopedSearch>) {
    state.rendering.set(true);
    state.question.set(None);
    state.confirmation.set_visible_child_name("none");
    state.editor_title.set_text(if previous.is_some() {
        "Edit saved search"
    } else if search.is_some() {
        "New saved search"
    } else {
        "Choose a saved search"
    });
    state.name.set_text(
        &previous
            .as_ref()
            .map(|entry| entry.name.clone())
            .unwrap_or_else(|| search.as_ref().map(default_name).unwrap_or_default()),
    );
    if let Some(search) = &search {
        state.query.set_text(&search.query);
        state.root.set_text(&search.root.to_string_lossy());
        state.scope.set_selected(match search.scope {
            SearchScope::ThisFolder => 0,
            SearchScope::Home => 1,
            SearchScope::MusicLibrary => 2,
        });
        state.group.set_selected(
            GROUPS
                .iter()
                .position(|group| *group == search.group)
                .unwrap_or(0) as u32,
        );
    } else {
        state.query.set_text("");
        state.root.set_text("");
        state.scope.set_selected(1);
        state.group.set_selected(0);
    }
    state.previous.replace(previous);
    state.context.replace(search);
    state.rendering.set(false);
    render_list(state);
    update_membership(state);
    update_controls(state);
}

fn default_name(search: &ScopedSearch) -> String {
    let name = if search.query.trim().is_empty() {
        group_name(search.group)
    } else {
        search.query.trim()
    };
    name.chars()
        .filter(|character| !character.is_control())
        .take(80)
        .collect()
}

fn group_name(group: SearchGroup) -> &'static str {
    match group {
        SearchGroup::All => "All",
        SearchGroup::Files => "Files",
        SearchGroup::Songs => "Songs",
        SearchGroup::Albums => "Albums",
        SearchGroup::Artists => "Artists",
    }
}

fn selected_scope(state: &State) -> SearchScope {
    match state.scope.selected() {
        0 => SearchScope::ThisFolder,
        2 => SearchScope::MusicLibrary,
        _ => SearchScope::Home,
    }
}

fn edited_uri(state: &State) -> Result<String, String> {
    let mut search = state
        .context
        .borrow()
        .clone()
        .ok_or("Choose a saved search or create a new one")?;
    search.scope = selected_scope(state);
    let root_text = state.root.text();
    let unchanged_root = root_text.as_str() == search.root.to_string_lossy();
    search.root = if search.scope != SearchScope::ThisFolder {
        search.scope.root(&search.root)
    } else if unchanged_root {
        search.root.clone()
    } else {
        PathBuf::from(root_text.as_str())
    };
    if !search.root.is_absolute() {
        return Err("Enter an absolute folder path, starting with /".into());
    }
    search.query = state.query.text().trim().to_owned();
    search.group = *GROUPS
        .get(state.group.selected() as usize)
        .ok_or("Choose All, Files or Songs")?;
    let uri = search.to_uri();
    ScopedSearch::from_uri(&uri)?;
    Ok(uri)
}

fn valid_name(state: &State) -> Result<String, String> {
    let name = state.name.text().trim().to_owned();
    if name.is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
        return Err("Enter a name of 1–80 characters without control characters".into());
    }
    Ok(name)
}

fn can_mutate(state: &State) -> bool {
    state.ready.get() && state.writable.get() && state.pending.borrow().is_none()
}

fn update_controls(state: &State) {
    let idle = state.pending.borrow().is_none() && state.question.get().is_none();
    let editable = can_mutate(state) && idle && state.context.borrow().is_some();
    state.editor.set_sensitive(editable);
    state
        .root
        .set_sensitive(selected_scope(state) == SearchScope::ThisFolder);
    state.list.set_sensitive(idle);
    state.create.set_sensitive(can_mutate(state) && idle);
    state
        .load
        .set_sensitive(idle && state.list.selected_row().is_some());
    state.reload.set_sensitive(idle);
    state
        .delete
        .set_sensitive(can_mutate(state) && idle && state.previous.borrow().is_some());
    state.open.set_sensitive(
        idle && state.previous.borrow().as_ref().is_some_and(|previous| {
            state.entries.borrow().contains(previous)
                && ScopedSearch::from_uri(&previous.uri).is_ok()
        }),
    );
    let valid = editable && valid_name(state).is_ok() && edited_uri(state).is_ok();
    state.save.set_sensitive(valid);
    state.save.set_label(if state.pending.borrow().is_some() {
        "Saving…"
    } else {
        "Save"
    });
    if !valid && editable {
        let error = valid_name(state).and_then(|_| edited_uri(state));
        state.save.set_tooltip_text(error.err().as_deref());
    } else {
        state
            .save
            .set_tooltip_text(Some("Save changes to storage (Ctrl+S)"));
    }
}

fn update_membership(state: &State) {
    let context = state.context.borrow();
    let text = match context.as_ref().and_then(|search| search.entity.as_ref()) {
        Some(SearchEntityKey::Album { artist, album }) => format!(
            "Album membership: {album} — {artist}. Kept when changing name, query or location; changing away from Songs requires confirmation."
        ),
        Some(SearchEntityKey::Artist(artist)) => format!(
            "Artist membership: {artist}. Kept when changing name, query or location; changing away from Songs requires confirmation."
        ),
        None => "No album or artist membership filter.".into(),
    };
    state.membership.set_text(&text);
    state.membership.set_tooltip_text(Some(&text));
}

fn message(state: &State, text: &str) {
    state.status.set_text(text);
    state.status.set_tooltip_text(Some(text));
}

fn save(state: &State) {
    if !can_mutate(state) || state.question.get().is_some() {
        return;
    }
    let values = valid_name(state).and_then(|name| edited_uri(state).map(|uri| (name, uri)));
    let (name, uri) = match values {
        Ok(values) => values,
        Err(error) => {
            message(state, &error);
            return;
        }
    };
    let previous = state.previous.borrow().clone();
    let (command, operation) = if let Some(previous) = previous {
        (
            Command::Update {
                previous: previous.clone(),
                name: name.clone(),
                uri: uri.clone(),
            },
            Operation::Update {
                previous,
                name,
                uri,
            },
        )
    } else {
        let existing_ids = state
            .entries
            .borrow()
            .iter()
            .map(|entry| entry.id)
            .collect();
        (
            Command::Create {
                name: name.clone(),
                uri: uri.clone(),
            },
            Operation::Create {
                name,
                uri,
                existing_ids,
            },
        )
    };
    submit(
        state,
        command,
        operation,
        "Saving to storage… Files are unchanged.",
    );
}

fn submit(state: &State, command: Command, operation: Operation, progress: &str) {
    let revision = state.store.revision();
    match state.store.send(command) {
        Ok(()) => {
            state.pending.replace(Some(Pending {
                revision,
                operation,
            }));
            message(state, progress);
        }
        Err(error) => message(
            state,
            &format!("Not saved: {error}. Your fields are unchanged."),
        ),
    }
    update_controls(state);
}

fn ask(state: &State, question: Question) {
    if !can_mutate(state) || state.question.get().is_some() {
        return;
    }
    let (text, action) = match question {
        Question::Delete => (
            "Delete this saved search? Only the saved name and query are removed. No files or folders will be deleted.",
            "Delete saved search",
        ),
        Question::ClearMembership(_) => (
            "Changing away from Songs clears the album or artist membership filter. Your typed query is kept. Clear this context?",
            "Clear membership",
        ),
    };
    state.question.set(Some(question));
    state.confirmation_text.set_text(text);
    state.confirmation_text.set_tooltip_text(Some(text));
    state.confirm.set_label(action);
    state.confirmation.set_visible_child_name("confirm");
    update_controls(state);
    state.cancel.grab_focus();
}

fn cancel_question(state: &State) {
    state.question.set(None);
    state.confirmation.set_visible_child_name("none");
    update_controls(state);
    state.name.grab_focus();
}

fn confirm(state: &State) {
    if !can_mutate(state) {
        return;
    }
    let question = state.question.take();
    state.confirmation.set_visible_child_name("none");
    match question {
        Some(Question::Delete) => {
            let previous = state.previous.borrow().clone();
            if let Some(previous) = previous {
                submit(
                    state,
                    Command::Delete(previous.clone()),
                    Operation::Delete(previous),
                    "Deleting saved query from storage… No files will be deleted.",
                );
            }
        }
        Some(Question::ClearMembership(selected)) => {
            if let Some(search) = state.context.borrow_mut().as_mut() {
                search.entity = None;
            }
            state.group.set_selected(selected);
            update_membership(state);
            message(
                state,
                "Album/artist context cleared in the editor. Save to keep this change; files and your typed query are unchanged.",
            );
            update_controls(state);
            state.group.grab_focus();
        }
        None => {}
    }
}

fn reload(state: &State) {
    if state.pending.borrow().is_some() || state.question.get().is_some() {
        return;
    }
    submit(
        state,
        Command::Reload,
        Operation::Reload,
        "Reloading saved searches… Your editor fields are kept.",
    );
}

fn open_selected(state: &State) {
    if !state.open.is_sensitive() {
        return;
    }
    let selected = state.previous.borrow().clone();
    if let Some(selected) = selected {
        (state.open_callback)(selected.uri);
        hide(state);
    }
}
