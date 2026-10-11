// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
    time::Duration,
};

use gtk::{gdk, glib, prelude::*};

use crate::adapters::LocalTextExtractionProvider;
mod results;
use crate::services::{
    ScopedSearch, SearchEvent, SearchFilterValues, SearchHandle, SearchItem, SearchScope,
    index_tree, search_filter_values,
};

const SEARCH_HELP: &str = "Search names, song titles, artists and albums in the selected scope\nHidden files and symlinks excluded · up to 40,000 entries / 32 MiB / 128 levels\nFilters: content:\"phrase\" · type:file · ext:pdf · in:Documents · modified:today";

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
    notice: gtk::Label,
    root: RefCell<PathBuf>,
    current_folder: RefCell<PathBuf>,
    scope: gtk::DropDown,
    scope_label: gtk::Label,
    refresh: gtk::Button,
    selected_path: RefCell<Option<PathBuf>>,
    rendering: Cell<bool>,
    visible_results: RefCell<Vec<SearchItem>>,
    search: RefCell<Option<SearchHandle>>,
    generation: Cell<u64>,
    activate: Rc<dyn Fn(SearchItem)>,
    dismiss: Rc<dyn Fn()>,
    open_results_button: gtk::Button,
    open_results: RefCell<Option<Rc<dyn Fn(String)>>>,
    filters: SearchFilterPanel,
}

impl Drop for SearchState {
    fn drop(&mut self) {
        self.filters.popover.unparent();
    }
}

#[derive(Clone)]
struct SearchFilterPanel {
    button: gtk::Button,
    popover: gtk::Popover,
    file_type: gtk::DropDown,
    modified: gtk::DropDown,
    extension: gtk::Entry,
    within: gtk::Entry,
    content_enabled: gtk::CheckButton,
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
            .placeholder_text("Search songs, artists, albums or files…")
            .hexpand(true)
            .build();
        field.add_css_class("search-field");
        field.set_max_length(1024);
        search_bar.append(&field);
        let filters = SearchFilterPanel::new();
        search_bar.append(&filters.button);
        panel.append(&search_bar);

        let scope_bar = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        scope_bar.add_css_class("search-bar");
        let scope = gtk::DropDown::from_strings(&["This folder", "Home", "Music library"]);
        scope.set_selected(2);
        scope.set_tooltip_text(Some(
            "Search scope — Music library covers all albums in ~/Music",
        ));
        let scope_label = gtk::Label::new(None);
        scope_label.add_css_class("search-result-path");
        scope_label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        scope_label.set_hexpand(true);
        scope_label.set_xalign(0.0);
        scope_bar.append(&scope);
        scope_bar.append(&scope_label);
        let refresh = gtk::Button::with_label("Refresh");
        refresh.add_css_class("search-filter-apply");
        refresh.set_tooltip_text(Some(
            "Rescan this scope for added or changed files (Ctrl+R)",
        ));
        scope_bar.append(&refresh);
        panel.append(&scope_bar);

        let notice = gtk::Label::new(None);
        notice.add_css_class("search-status");
        notice.set_wrap(true);
        notice.set_xalign(0.0);
        notice.set_visible(false);
        panel.append(&notice);

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
        let filter_hint = gtk::Label::new(Some("Ctrl+F  filters"));
        navigation.add_css_class("search-hint");
        open.add_css_class("search-hint");
        filter_hint.add_css_class("search-hint");
        footer.append(&navigation);
        footer.append(&open);
        footer.append(&filter_hint);
        let open_results_button = gtk::Button::with_label("Show results in tab");
        open_results_button.add_css_class("search-filter-apply");
        open_results_button.set_visible(false);
        footer.append(&open_results_button);
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
            notice,
            refresh,
            root: RefCell::new(PathBuf::new()),
            current_folder: RefCell::new(glib::home_dir()),
            scope,
            scope_label,
            selected_path: RefCell::new(None),
            rendering: Cell::new(false),
            visible_results: RefCell::new(Vec::new()),
            search: RefCell::new(None),
            generation: Cell::new(0),
            activate,
            dismiss,
            open_results_button,
            open_results: RefCell::new(None),
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
        let opened = Rc::downgrade(&state);
        state.open_results_button.connect_clicked(move |_| {
            if let Some(state) = opened.upgrade() {
                let callback = state.open_results.borrow().clone();
                if let Some(callback) = callback {
                    let uri = SearchDialog {
                        state: state.clone(),
                    }
                    .search_uri();
                    hide(&state);
                    callback(uri);
                }
            }
        });
        let scoped = Rc::downgrade(&state);
        let refreshed = Rc::downgrade(&state);
        state.refresh.connect_clicked(move |_| {
            if let Some(state) = refreshed.upgrade() {
                restart_search(&state);
            }
        });
        state.scope.connect_selected_notify(move |_| {
            if let Some(state) = scoped.upgrade() {
                if state.layer.is_visible() {
                    restart_search(&state);
                }
            }
        });
        let selected = Rc::downgrade(&state);
        state.list.connect_row_selected(move |_, row| {
            if let Some(state) = selected.upgrade() {
                if !state.rendering.get() {
                    let path = row
                        .and_then(|row| usize::try_from(row.index()).ok())
                        .and_then(|position| {
                            state
                                .visible_results
                                .borrow()
                                .get(position)
                                .map(|item| item.path.clone())
                        });
                    state.selected_path.replace(path);
                }
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
                && matches!(key, gdk::Key::r | gdk::Key::R)
            {
                restart_search(&state);
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
        self.state.current_folder.replace(root);
        self.state.search.borrow_mut().take();
        if self.state.field.text() != query {
            self.state.field.set_text(query);
        }
        self.state.field.set_position(-1);
        self.state.layer.set_visible(true);
        restart_search(&self.state);
        self.state.field.grab_focus();
    }

    pub fn show_music(&self) {
        self.state.scope.set_selected(2);
        self.show(SearchScope::MusicLibrary.root(&glib::home_dir()));
    }

    pub fn search_uri(&self) -> String {
        let folder = self.state.current_folder.borrow().clone();
        ScopedSearch::new(
            selected_scope(self.state.scope.selected()),
            &folder,
            &self.state.field.text(),
        )
        .to_uri()
    }

    pub fn set_open_results(&self, callback: Rc<dyn Fn(String)>) {
        self.state.open_results.replace(Some(callback));
        self.state.open_results_button.set_visible(true);
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
        let title_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let title = gtk::Label::new(Some("SEARCH FILTERS"));
        title.add_css_class("search-filter-title");
        title.set_xalign(0.0);
        title.set_hexpand(true);
        let shortcut = gtk::Label::new(Some("Ctrl+F"));
        shortcut.add_css_class("search-filter-shortcut");
        title_row.append(&title);
        title_row.append(&shortcut);
        content.append(&title_row);

        let file_type = gtk::DropDown::from_strings(&["Any type", "Files", "Folders"]);
        append_filter_control(&content, "Type", &file_type);
        let modified = gtk::DropDown::from_strings(&["Any time", "Today", "Yesterday"]);
        append_filter_control(&content, "Modified", &modified);
        let extension = filter_entry("pdf");
        append_filter_control(&content, "Extension", &extension);
        let within = filter_entry("Documents or Projects/hermes");
        append_filter_control(&content, "Within folder", &within);
        let content_options = gtk::Box::new(gtk::Orientation::Vertical, 5);
        let content_enabled = gtk::CheckButton::with_label("Search for any text inside files");
        content_enabled.add_css_class("search-content-toggle");
        let content_query = filter_entry("Words or an exact phrase");
        content_query.set_sensitive(false);
        let content_note = gtk::Label::new(Some(
            "Text, code, data, and sandboxed PDF text; bounded to protect system performance",
        ));
        content_note.add_css_class("search-content-note");
        content_note.set_xalign(0.0);
        content_note.set_wrap(true);
        content_options.append(&content_enabled);
        content_options.append(&content_query);
        content_options.append(&content_note);
        append_filter_control(&content, "Contents", &content_options);
        let enabled_content = content_query.clone();
        content_enabled.connect_toggled(move |toggle| {
            enabled_content.set_sensitive(toggle.is_active());
            if toggle.is_active() {
                enabled_content.grab_focus();
            }
        });

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
            content_enabled,
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
        self.content_enabled.set_active(values.content.is_some());
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
        if self.content_enabled.is_active() {
            push_filter(&mut parts, "content", &self.content.text());
        }
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
    state.selected_path.replace(None);
    state.visible_results.borrow_mut().clear();
    state.rendering.set(true);
    while let Some(child) = state.list.first_child() {
        state.list.remove(&child);
    }
    state.rendering.set(false);
    state.notice.set_visible(false);
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

fn render_results(
    state: &Rc<SearchState>,
    results: Vec<SearchItem>,
    indexing: bool,
    notice: Option<&str>,
) {
    let selected_path = state.selected_path.borrow().clone();
    let awaiting_selection = indexing
        && selected_path
            .as_ref()
            .is_some_and(|path| !results.iter().any(|item| &item.path == path));
    let selected = selection_position(&results, selected_path.as_deref());
    let root = state.root.borrow().clone();
    let changed = *state.visible_results.borrow() != results;
    let rows: Vec<_> = if changed {
        results
            .iter()
            .map(|item| results::row(item, &root))
            .collect()
    } else {
        Vec::new()
    };
    let has_results = !results.is_empty();
    state.visible_results.replace(results);
    state.rendering.set(true);
    let progress = if indexing {
        "Indexing selected scope — results update as song tags arrive"
    } else {
        ""
    };
    let notice = [progress, notice.unwrap_or_default()]
        .into_iter()
        .filter(|text| !text.is_empty())
        .collect::<Vec<_>>()
        .join(" · ");
    state.notice.set_text(&notice);
    state.notice.set_visible(!notice.is_empty());
    if changed {
        while let Some(child) = state.list.first_child() {
            state.list.remove(&child);
        }
        for row in rows {
            state.list.append(&row);
        }
    }
    state
        .results
        .set_visible_child_name(if has_results { "results" } else { "status" });
    if let Some(row) = selected.and_then(|position| state.list.row_at_index(position as i32)) {
        state.list.select_row(Some(&row));
    } else {
        state
            .status
            .set_text(if state.field.text().trim().is_empty() {
                SEARCH_HELP
            } else if indexing {
                "Searching…"
            } else {
                "No matching files, folders or songs in this scope"
            });
    }
    state.rendering.set(false);
    let selected_path = selected.and_then(|position| {
        state
            .visible_results
            .borrow()
            .get(position)
            .map(|item| item.path.clone())
    });
    if !awaiting_selection {
        state.selected_path.replace(selected_path);
    }
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

fn selected_scope(position: u32) -> SearchScope {
    match position {
        0 => SearchScope::ThisFolder,
        1 => SearchScope::Home,
        _ => SearchScope::MusicLibrary,
    }
}

fn selection_position(results: &[SearchItem], selected: Option<&Path>) -> Option<usize> {
    selected
        .and_then(|selected| results.iter().position(|item| item.path == selected))
        .or_else(|| (!results.is_empty()).then_some(0))
}

fn restart_search(state: &Rc<SearchState>) {
    state.generation.set(state.generation.get().wrapping_add(1));
    let generation = state.generation.get();
    state.search.borrow_mut().take();
    let folder = state.current_folder.borrow().clone();
    let root = selected_scope(state.scope.selected()).root(&folder);
    if *state.root.borrow() != root {
        state.selected_path.replace(None);
        state.visible_results.borrow_mut().clear();
        state.rendering.set(true);
        while let Some(child) = state.list.first_child() {
            state.list.remove(&child);
        }
        state.rendering.set(false);
        state.results.set_visible_child_name("status");
        state.status.set_text("Searching selected scope…");
    }
    state.root.replace(root.clone());
    state.scope_label.set_text(&root.to_string_lossy());
    state
        .scope_label
        .set_tooltip_text(Some(&root.to_string_lossy()));
    let (handle, receiver) = index_tree(root, Arc::new(LocalTextExtractionProvider));
    handle.query(&state.field.text());
    state.search.replace(Some(handle));
    let weak = Rc::downgrade(state);
    let _poll = glib::timeout_add_local(Duration::from_millis(16), move || {
        let Some(state) = weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        if !state.layer.is_visible() || state.generation.get() != generation {
            return glib::ControlFlow::Break;
        }
        if let Some(SearchEvent::Results {
            query,
            items,
            indexing,
            notice,
        }) = receiver.try_recv()
        {
            if query == state.field.text().trim() {
                render_results(&state, items, indexing, notice.as_deref());
            }
        }
        glib::ControlFlow::Continue
    });
}

#[cfg(test)]
mod tests;
