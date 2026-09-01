// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::Rc,
    sync::mpsc::TryRecvError,
    time::Duration,
};

use gtk::{gdk, glib, prelude::*};

use crate::services::{
    SearchEvent, SearchFilterValues, SearchHandle, SearchItem, index_tree, search_filter_values,
};

const MAX_RESULT_UPDATES_PER_FRAME: usize = 8;
const SEARCH_HELP: &str = "Type to search the whole tree\nFilters: content:\"phrase\" · type:file · ext:pdf · in:Documents · modified:today";

#[derive(Clone)]
pub struct SearchDialog {
    state: Rc<SearchState>,
}

struct SearchState {
    layer: gtk::Box,
    field: gtk::Entry,
    list: gtk::ListBox,
    results: gtk::Stack,
    status: gtk::Label,
    root: RefCell<PathBuf>,
    visible_results: RefCell<Vec<SearchItem>>,
    search: RefCell<Option<SearchHandle>>,
    generation: Cell<u64>,
    activate: Rc<dyn Fn(SearchItem)>,
    dismiss: Rc<dyn Fn()>,
    filters: SearchFilterPanel,
}

#[derive(Clone)]
struct SearchFilterPanel {
    button: gtk::Button,
    popover: gtk::Popover,
    file_type: gtk::DropDown,
    modified: gtk::DropDown,
    extension: gtk::Entry,
    within: gtk::Entry,
    content: gtk::Entry,
    apply: gtk::Button,
    clear: gtk::Button,
}

impl SearchDialog {
    pub fn new(activate: Rc<dyn Fn(SearchItem)>, dismiss: Rc<dyn Fn()>) -> Self {
        let layer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        layer.add_css_class("search-backdrop");
        layer.add_css_class("app-modal-layer");
        layer.set_halign(gtk::Align::Fill);
        layer.set_valign(gtk::Align::Fill);
        layer.set_hexpand(true);
        layer.set_vexpand(true);
        layer.set_focusable(true);
        layer.set_visible(false);

        let panel = gtk::Box::new(gtk::Orientation::Vertical, 0);
        panel.add_css_class("search-dialog");
        panel.set_halign(gtk::Align::Center);
        panel.set_valign(gtk::Align::Center);
        panel.set_size_request(760, 452);
        panel.set_vexpand(false);
        panel.set_overflow(gtk::Overflow::Hidden);

        let search_bar = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        search_bar.add_css_class("search-bar");
        search_bar.append(&crate::assets::primary_icon(
            crate::assets::icons::SEARCH,
            16,
        ));
        let field = gtk::Entry::builder()
            .placeholder_text("Search files and folders…")
            .hexpand(true)
            .build();
        field.add_css_class("search-field");
        search_bar.append(&field);
        let filters = SearchFilterPanel::new();
        search_bar.append(&filters.button);
        panel.append(&search_bar);

        let status = gtk::Label::new(Some(SEARCH_HELP));
        status.add_css_class("search-status");
        status.set_wrap(true);

        let list = gtk::ListBox::new();
        list.add_css_class("search-results");
        list.set_selection_mode(gtk::SelectionMode::Single);
        list.set_activate_on_single_click(false);
        let scroller = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .vexpand(true)
            .child(&list)
            .build();
        scroller.add_css_class("search-results-scroll");
        let results = gtk::Stack::new();
        results.add_css_class("search-results-stack");
        results.set_size_request(-1, 360);
        results.add_named(&status, Some("status"));
        results.add_named(&scroller, Some("results"));
        results.set_visible_child_name("status");
        panel.append(&results);

        let footer = gtk::Box::new(gtk::Orientation::Horizontal, 18);
        footer.add_css_class("search-footer");
        let navigation = gtk::Label::new(Some("↑↓  navigate"));
        let open = gtk::Label::new(Some("↵  open / reveal"));
        navigation.add_css_class("search-hint");
        open.add_css_class("search-hint");
        footer.append(&navigation);
        footer.append(&open);
        panel.append(&footer);
        let top_spacer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        top_spacer.set_vexpand(true);
        let bottom_spacer = gtk::Box::new(gtk::Orientation::Vertical, 0);
        bottom_spacer.set_vexpand(true);
        layer.append(&top_spacer);
        layer.append(&panel);
        layer.append(&bottom_spacer);

        let state = Rc::new(SearchState {
            layer,
            field,
            list,
            results,
            status,
            root: RefCell::new(PathBuf::new()),
            visible_results: RefCell::new(Vec::new()),
            search: RefCell::new(None),
            generation: Cell::new(0),
            activate,
            dismiss,
            filters,
        });

        let shown_filters = Rc::downgrade(&state);
        state.filters.button.connect_clicked(move |_| {
            if let Some(state) = shown_filters.upgrade() {
                state.filters.show(&state.field.text());
            }
        });
        let applied_filters = Rc::downgrade(&state);
        state.filters.apply.connect_clicked(move |_| {
            if let Some(state) = applied_filters.upgrade() {
                let query = state.filters.query(&state.field.text());
                state.field.set_text(&query);
                state.field.set_position(-1);
                state.filters.popover.popdown();
                state.field.grab_focus();
            }
        });
        let cleared_filters = Rc::downgrade(&state);
        state.filters.clear.connect_clicked(move |_| {
            if let Some(state) = cleared_filters.upgrade() {
                let (terms, _) = search_filter_values(&state.field.text());
                state.filters.set_values(&SearchFilterValues::default());
                state.filters.update_active(false);
                state.field.set_text(&terms);
                state.field.set_position(-1);
                state.filters.popover.popdown();
                state.field.grab_focus();
            }
        });

        let changed = Rc::downgrade(&state);
        state.field.connect_changed(move |field| {
            if let Some(state) = changed.upgrade() {
                begin_query(&state, &field.text());
            }
        });
        let activated = Rc::downgrade(&state);
        state.list.connect_row_activated(move |_, row| {
            if let Some(state) = activated.upgrade() {
                activate_position(&state, row.index());
            }
        });
        let keys = gtk::EventControllerKey::new();
        keys.set_propagation_phase(gtk::PropagationPhase::Capture);
        let keyed = Rc::downgrade(&state);
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            let Some(state) = keyed.upgrade() else {
                return glib::Propagation::Proceed;
            };
            if key == gdk::Key::Escape {
                if state.filters.popover.is_visible() {
                    state.filters.popover.popdown();
                    state.field.grab_focus();
                    return glib::Propagation::Stop;
                }
                hide(&state);
                return glib::Propagation::Stop;
            }
            if modifiers.contains(gdk::ModifierType::CONTROL_MASK)
                && matches!(key, gdk::Key::f | gdk::Key::F)
            {
                state.filters.show(&state.field.text());
                return glib::Propagation::Stop;
            }
            if modifiers.intersects(
                gdk::ModifierType::CONTROL_MASK
                    | gdk::ModifierType::ALT_MASK
                    | gdk::ModifierType::SUPER_MASK,
            ) {
                return glib::Propagation::Proceed;
            }
            if matches!(key, gdk::Key::Down | gdk::Key::Up) {
                move_selection(&state, if key == gdk::Key::Down { 1 } else { -1 });
                return glib::Propagation::Stop;
            }
            if matches!(key, gdk::Key::Return | gdk::Key::KP_Enter)
                && let Some(row) = state.list.selected_row()
            {
                activate_position(&state, row.index());
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        state.layer.add_controller(keys);

        Self { state }
    }

    pub fn widget(&self) -> gtk::Widget {
        self.state.layer.clone().upcast()
    }

    pub fn show(&self, root: PathBuf) {
        self.show_query(root, "");
    }

    pub fn show_query(&self, root: PathBuf, query: &str) {
        self.state.generation.set(self.state.generation.get() + 1);
        let generation = self.state.generation.get();
        self.state.search.borrow_mut().take();
        self.state.root.replace(root.clone());
        self.state.visible_results.borrow_mut().clear();
        self.state.field.set_text("");
        self.state.status.set_visible(true);
        self.state.status.set_text(SEARCH_HELP);
        self.state.layer.set_visible(true);
        self.state.field.grab_focus();

        let (handle, receiver) = index_tree(root);
        self.state.search.replace(Some(handle));
        self.state.field.set_text(query);
        self.state.field.set_position(-1);
        let weak = Rc::downgrade(&self.state);
        let _poll = glib::timeout_add_local(Duration::from_millis(16), move || {
            let Some(state) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if !state.layer.is_visible() || state.generation.get() != generation {
                return glib::ControlFlow::Break;
            }
            let mut latest = None;
            for _ in 0..MAX_RESULT_UPDATES_PER_FRAME {
                match receiver.try_recv() {
                    Ok(event) => latest = Some(event),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return glib::ControlFlow::Break,
                }
            }
            if let Some(SearchEvent::Results {
                query,
                items,
                indexing,
            }) = latest
                && query == state.field.text().trim()
            {
                render_results(&state, items, indexing);
            }
            glib::ControlFlow::Continue
        });
    }

    pub fn hide(&self) {
        hide(&self.state);
    }

    pub fn is_visible(&self) -> bool {
        self.state.layer.is_visible()
    }
}

impl SearchFilterPanel {
    fn new() -> Self {
        let button = gtk::Button::builder()
            .tooltip_text("Search filters (Ctrl+F)")
            .build();
        button.set_child(Some(&crate::assets::primary_icon(
            crate::assets::icons::FUNNEL,
            16,
        )));
        button.add_css_class("search-filter-button");

        let content = gtk::Box::new(gtk::Orientation::Vertical, 10);
        content.add_css_class("search-filter-panel");
        let title = gtk::Label::new(Some("SEARCH FILTERS"));
        title.add_css_class("search-filter-title");
        title.set_xalign(0.0);
        content.append(&title);

        let file_type = gtk::DropDown::from_strings(&["Any type", "Files", "Folders"]);
        append_filter_control(&content, "Type", &file_type);
        let modified = gtk::DropDown::from_strings(&["Any time", "Today", "Yesterday"]);
        append_filter_control(&content, "Modified", &modified);
        let extension = filter_entry("pdf");
        append_filter_control(&content, "Extension", &extension);
        let within = filter_entry("Documents or Projects/hermes");
        append_filter_control(&content, "Within folder", &within);
        let content_query = filter_entry("Words or an exact phrase");
        append_filter_control(&content, "File contains", &content_query);

        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        actions.set_halign(gtk::Align::End);
        let clear = gtk::Button::with_label("Clear");
        clear.add_css_class("search-filter-clear");
        let apply = gtk::Button::with_label("Apply filters");
        apply.add_css_class("search-filter-apply");
        actions.append(&clear);
        actions.append(&apply);
        content.append(&actions);

        let popover = gtk::Popover::builder()
            .child(&content)
            .autohide(true)
            .has_arrow(false)
            .build();
        popover.add_css_class("search-filter-popover");
        popover.set_parent(&button);

        Self {
            button,
            popover,
            file_type,
            modified,
            extension,
            within,
            content: content_query,
            apply,
            clear,
        }
    }

    fn show(&self, query: &str) {
        let (_, values) = search_filter_values(query);
        self.set_values(&values);
        self.popover.popup();
        self.file_type.grab_focus();
    }

    fn set_values(&self, values: &SearchFilterValues) {
        self.file_type.set_selected(match values.directory {
            Some(false) => 1,
            Some(true) => 2,
            None => 0,
        });
        self.modified
            .set_selected(match values.modified.as_deref() {
                Some("today") => 1,
                Some("yesterday") => 2,
                _ => 0,
            });
        self.extension
            .set_text(values.extension.as_deref().unwrap_or_default());
        self.within
            .set_text(values.within.as_deref().unwrap_or_default());
        self.content
            .set_text(values.content.as_deref().unwrap_or_default());
        self.update_active(values != &SearchFilterValues::default());
    }

    fn query(&self, current: &str) -> String {
        let (terms, _) = search_filter_values(current);
        let mut parts = Vec::new();
        if !terms.is_empty() {
            parts.push(terms);
        }
        match self.file_type.selected() {
            1 => parts.push("type:file".to_owned()),
            2 => parts.push("type:folder".to_owned()),
            _ => {}
        }
        push_filter(&mut parts, "ext", &self.extension.text());
        push_filter(&mut parts, "in", &self.within.text());
        match self.modified.selected() {
            1 => parts.push("modified:today".to_owned()),
            2 => parts.push("modified:yesterday".to_owned()),
            _ => {}
        }
        push_filter(&mut parts, "content", &self.content.text());
        let active = parts.iter().any(|part| part.contains(':'));
        self.update_active(active);
        parts.join(" ")
    }

    fn update_active(&self, active: bool) {
        if active {
            self.button.add_css_class("active");
        } else {
            self.button.remove_css_class("active");
        }
    }
}

fn filter_entry(placeholder: &str) -> gtk::Entry {
    let entry = gtk::Entry::builder()
        .placeholder_text(placeholder)
        .hexpand(true)
        .build();
    entry.add_css_class("search-filter-entry");
    entry
}

fn append_filter_control(container: &gtk::Box, label: &str, control: &impl IsA<gtk::Widget>) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let label = gtk::Label::new(Some(label));
    label.add_css_class("search-filter-label");
    label.set_xalign(0.0);
    label.set_width_chars(13);
    row.append(&label);
    row.append(control);
    container.append(&row);
}

fn push_filter(parts: &mut Vec<String>, name: &str, value: &str) {
    let value = value.trim();
    if value.is_empty() {
        return;
    }
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    parts.push(format!("{name}:\"{escaped}\""));
}

fn begin_query(state: &Rc<SearchState>, query: &str) {
    while let Some(child) = state.list.first_child() {
        state.list.remove(&child);
    }
    state.visible_results.borrow_mut().clear();
    state.results.set_visible_child_name("status");
    if query.trim().is_empty() {
        state.status.set_text(SEARCH_HELP);
    } else {
        state.status.set_text("Searching…");
    }
    if let Some(search) = state.search.borrow().as_ref() {
        search.query(query);
    }
}

fn render_results(state: &Rc<SearchState>, results: Vec<SearchItem>, indexing: bool) {
    while let Some(child) = state.list.first_child() {
        state.list.remove(&child);
    }
    let root = state.root.borrow();
    for item in &results {
        state.list.append(&result_row(item, &root));
    }
    let has_results = !results.is_empty();
    state.visible_results.replace(results);
    state
        .results
        .set_visible_child_name(if has_results { "results" } else { "status" });
    if let Some(first) = state.list.row_at_index(0) {
        state.list.select_row(Some(&first));
    } else {
        state.status.set_text(if indexing {
            "Searching…"
        } else {
            "No matching files or folders"
        });
    }
}

fn result_row(item: &SearchItem, root: &Path) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.add_css_class("search-result");
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    content.append(&crate::assets::primary_icon(
        if item.is_directory {
            crate::assets::icons::FOLDER
        } else {
            crate::assets::icons::DOCUMENTS
        },
        19,
    ));
    let labels = gtk::Box::new(gtk::Orientation::Vertical, 2);
    labels.set_hexpand(true);
    let name = gtk::Label::new(Some(&item.name));
    name.add_css_class("search-result-name");
    name.set_xalign(0.0);
    name.set_ellipsize(gtk::pango::EllipsizeMode::End);
    let relative = item.path.strip_prefix(root).unwrap_or(&item.path);
    let path = gtk::Label::new(Some(&relative.to_string_lossy()));
    path.add_css_class("search-result-path");
    path.set_xalign(0.0);
    path.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
    labels.append(&name);
    labels.append(&path);
    content.append(&labels);
    row.set_child(Some(&content));
    row
}

fn move_selection(state: &SearchState, direction: i32) {
    let count = state.visible_results.borrow().len() as i32;
    if count == 0 {
        return;
    }
    let current = state.list.selected_row().map_or(-1, |row| row.index());
    let next = (current + direction).clamp(0, count - 1);
    if let Some(row) = state.list.row_at_index(next) {
        state.list.select_row(Some(&row));
        row.grab_focus();
    }
}

fn activate_position(state: &Rc<SearchState>, position: i32) {
    let Some(item) = usize::try_from(position)
        .ok()
        .and_then(|position| state.visible_results.borrow().get(position).cloned())
    else {
        return;
    };
    hide(state);
    (state.activate)(item);
}

fn hide(state: &SearchState) {
    state.generation.set(state.generation.get() + 1);
    state.search.borrow_mut().take();
    state.layer.set_visible(false);
    (state.dismiss)();
}
