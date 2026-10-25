// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
    time::Duration,
};

use gtk::{gdk, glib, prelude::*};

use crate::adapters::LocalTextExtractionProvider;
mod results;
use crate::services::{
    InferredFilterChip, ScopedSearch, SearchCounts, SearchEntityKey, SearchEvent,
    SearchFilterValues, SearchGroup, SearchGroups, SearchHandle, SearchItem, SearchPage,
    SearchResult, SearchResultKey, SearchScope, index_tree, search_filter_values, search_filter_terms,
};

const SEARCH_HELP: &str = "Find files, folders, songs, artists and albums across Home\nChoose This folder to search another drive or a specific location\nUse Filters for types, dates and optional document content\nHidden files and symlinks are excluded";
const SEARCH_NOTICE_HINT: &str = "Names and metadata · Contents optional";
const SEARCH_TYPES: [&str; 9] = [
    "All",
    "Files",
    "Folders",
    "Songs",
    "Videos",
    "Images",
    "Documents",
    "Apps",
    "Archives",
];
const SEARCH_CATEGORIES: [&str; 6] = [
    "audio",
    "video",
    "image",
    "document",
    "application",
    "archive",
];
const SEARCH_GROUPS: [SearchGroup; 5] = [
    SearchGroup::All,
    SearchGroup::Files,
    SearchGroup::Songs,
    SearchGroup::Albums,
    SearchGroup::Artists,
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OmnibarCommand {
    pub id: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    pub shortcut: Option<&'static str>,
    pub icon: &'static str,
}

#[derive(Clone)]
pub struct SearchDialog {
    state: Rc<SearchState>,
}

struct SearchState {
    layer: gtk::Box,
    field: gtk::Entry,
    interpretation_slot: gtk::Box,
    interpretation_view: gtk::ScrolledWindow,
    list: gtk::ListBox,
    results: gtk::Stack,
    status: gtk::Label,
    notice: gtk::Label,
    #[cfg(test)]
    notice_slot: gtk::Box,
    root: RefCell<PathBuf>,
    current_folder: RefCell<PathBuf>,
    scope: gtk::DropDown,
    scope_label: gtk::Label,
    refresh: gtk::Button,
    save_button: gtk::Button,
    save_search: RefCell<Option<Rc<dyn Fn(String)>>>,
    selected_key: RefCell<Option<SearchResultKey>>,
    rendering: Cell<bool>,
    visible_results: RefCell<Vec<SearchResult>>,
    visible_commands: RefCell<Vec<OmnibarCommand>>,
    command_handler: RefCell<Option<Rc<dyn Fn(&str)>>>,
    group: Cell<SearchGroup>,
    entity_scope: RefCell<Option<EntityScope>>,
    grouped_headers: Cell<bool>,
    back_button: gtk::Button,
    search: RefCell<Option<SearchHandle>>,
    session_generation: Cell<u64>,
    expected_query_generation: Cell<Option<u64>>,
    page: Cell<Option<SearchPage>>,
    more_button: gtk::Button,
    activate: Rc<dyn Fn(SearchItem)>,
    dismiss: Rc<dyn Fn()>,
    open_results_button: gtk::Button,
    open_results: RefCell<Option<Rc<dyn Fn(String)>>>,
    filters: SearchFilterPanel,
    categories: Vec<gtk::ToggleButton>,
}

struct EntityScope {
    key: SearchEntityKey,
    previous_group: SearchGroup,
    previous_selection: Option<SearchResultKey>,
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
    artist: gtk::Entry,
    album: gtk::Entry,
    title: gtk::Entry,
    content_enabled: gtk::CheckButton,
    content: gtk::Entry,
    apply: gtk::Button,
    clear: gtk::Button,
}

pub fn all_omnibar_commands() -> Vec<OmnibarCommand> {
    vec![
        OmnibarCommand {
            id: "home",
            title: "Go to Home Dashboard",
            description: "Open Home start page with Quick Access & Recent Files",
            shortcut: None,
            icon: crate::assets::icons::HOME,
        },
        OmnibarCommand {
            id: "settings",
            title: "Open Preferences / Settings",
            description: "Customize theme, audio playback, layouts, and display options",
            shortcut: Some("Ctrl+,"),
            icon: crate::assets::icons::PALETTE,
        },
        OmnibarCommand {
            id: "view_columns",
            title: "View Mode: Columns",
            description: "Switch active browser view to Miller Columns",
            shortcut: Some("Alt+1"),
            icon: crate::assets::icons::LIST,
        },
        OmnibarCommand {
            id: "view_explorer",
            title: "View Mode: Explorer",
            description: "Switch active browser view to Explorer tree",
            shortcut: Some("Alt+2"),
            icon: crate::assets::icons::FOLDER,
        },
        OmnibarCommand {
            id: "view_grid",
            title: "View Mode: Grid",
            description: "Switch active browser view to Icon Grid",
            shortcut: Some("Alt+3"),
            icon: crate::assets::icons::GRID,
        },
        OmnibarCommand {
            id: "view_list",
            title: "View Mode: List",
            description: "Switch active browser view to Detailed List",
            shortcut: Some("Alt+4"),
            icon: crate::assets::icons::LIST,
        },
        OmnibarCommand {
            id: "density_compact",
            title: "Density: Compact",
            description: "Set compact item spacing",
            shortcut: None,
            icon: crate::assets::icons::ROWS,
        },
        OmnibarCommand {
            id: "density_comfortable",
            title: "Density: Comfortable",
            description: "Set spacious item spacing",
            shortcut: None,
            icon: crate::assets::icons::ROWS,
        },
        OmnibarCommand {
            id: "toggle_hidden",
            title: "Toggle Hidden Files",
            description: "Show or hide dotfiles and hidden folders",
            shortcut: Some("Ctrl+H"),
            icon: crate::assets::icons::EYE,
        },
        OmnibarCommand {
            id: "toggle_sidebar",
            title: "Toggle Sidebar",
            description: "Show or hide the places sidebar",
            shortcut: Some("Ctrl+B"),
            icon: crate::assets::icons::PANEL_LEFT,
        },
        OmnibarCommand {
            id: "toggle_folder_tree",
            title: "Toggle Folder Tree",
            description: "Show or hide the folder tree hierarchy panel",
            shortcut: None,
            icon: crate::assets::icons::FOLDER,
        },
        OmnibarCommand {
            id: "new_tab",
            title: "New Tab",
            description: "Open a new navigation tab",
            shortcut: Some("Ctrl+T"),
            icon: crate::assets::icons::PLUS,
        },
        OmnibarCommand {
            id: "close_tab",
            title: "Close Tab",
            description: "Close the active navigation tab",
            shortcut: Some("Ctrl+W"),
            icon: crate::assets::icons::X,
        },
        OmnibarCommand {
            id: "music_player",
            title: "Music Player",
            description: "Toggle integrated music player and library",
            shortcut: Some("Ctrl+M"),
            icon: crate::assets::icons::MUSIC,
        },
        OmnibarCommand {
            id: "empty_trash",
            title: "Empty Trash",
            description: "Permanently delete files in trash",
            shortcut: None,
            icon: crate::assets::icons::TRASH,
        },
    ]
}

pub(crate) fn matching_commands(filter: &str) -> Vec<OmnibarCommand> {
    let all = all_omnibar_commands();
    let term = filter.trim().to_ascii_lowercase();
    if term.is_empty() {
        return all;
    }
    all.into_iter()
        .filter(|cmd| {
            cmd.title.to_ascii_lowercase().contains(&term)
                || cmd.description.to_ascii_lowercase().contains(&term)
                || cmd.id.contains(&term)
        })
        .collect()
}

fn build_command_row(cmd: &OmnibarCommand) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.add_css_class("omnibar-command-row");

    let h_box = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    h_box.set_margin_start(12);
    h_box.set_margin_end(12);
    h_box.set_margin_top(8);
    h_box.set_margin_bottom(8);

    let icon = crate::assets::primary_icon(cmd.icon, 20);
    icon.add_css_class("omnibar-command-icon");
    h_box.append(&icon);

    let text_col = gtk::Box::new(gtk::Orientation::Vertical, 2);
    text_col.set_hexpand(true);

    let title_label = gtk::Label::new(Some(cmd.title));
    title_label.add_css_class("omnibar-command-title");
    title_label.set_xalign(0.0);

    let desc_label = gtk::Label::new(Some(cmd.description));
    desc_label.add_css_class("omnibar-command-desc");
    desc_label.set_xalign(0.0);

    text_col.append(&title_label);
    text_col.append(&desc_label);
    h_box.append(&text_col);

    if let Some(sc) = cmd.shortcut {
        let sc_label = gtk::Label::new(Some(sc));
        sc_label.add_css_class("omnibar-command-shortcut");
        sc_label.set_valign(gtk::Align::Center);
        h_box.append(&sc_label);
    }

    row.set_child(Some(&h_box));
    row
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
            .placeholder_text("Search files, music, or type > for commands…")
            .hexpand(true)
            .build();
        field.add_css_class("search-field");
        field.set_max_length(1024);
        search_bar.append(&field);
        let filters = SearchFilterPanel::new();
        search_bar.append(&filters.button);
        panel.append(&search_bar);

        let interpretation_slot = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        interpretation_slot.add_css_class("search-interpretation-slot");
        interpretation_slot.set_valign(gtk::Align::Center);
        interpretation_slot.set_hexpand(false);
        interpretation_slot.set_visible(false);
        let interpretation_view = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::External)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .propagate_natural_width(true)
            .max_content_width(420)
            .min_content_width(0)
            .hexpand(false)
            .child(&interpretation_slot)
            .build();
        interpretation_view.add_css_class("search-interpretation-view");
        interpretation_view.set_visible(false);
        if let Some(viewport) = interpretation_view.child().and_downcast::<gtk::Viewport>() {
            viewport.set_scroll_to_focus(true);
        }

        let scope_bar = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        scope_bar.add_css_class("search-scope-bar");
        let scope = gtk::DropDown::from_strings(&["This folder", "Home", "Music library"]);
        scope.set_selected(1);
        scope.set_tooltip_text(Some(
            "Home searches across your files and Music. This folder searches the current location, including other drives.",
        ));
        let scope_label = gtk::Label::new(None);
        scope_label.add_css_class("search-result-path");
        scope_label.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        scope_label.set_max_width_chars(1);
        scope_label.set_hexpand(true);
        scope_label.set_xalign(0.0);
        scope_bar.append(&scope);
        scope_bar.append(&scope_label);
        let back_button = gtk::Button::with_label("Back");
        back_button.add_css_class("search-filter-clear");
        back_button.set_size_request(64, 32);
        back_button.set_valign(gtk::Align::Center);
        back_button.set_sensitive(false);
        back_button.set_tooltip_text(Some("Back to overall search results"));
        scope_bar.append(&back_button);
        let save_button = gtk::Button::with_label("Save search");
        save_button.add_css_class("search-filter-clear");
        save_button.set_size_request(100, 32);
        save_button.set_valign(gtk::Align::Center);
        save_button.set_sensitive(false);
        save_button.set_visible(false);
        scope_bar.append(&save_button);
        let refresh = gtk::Button::with_label("Refresh");
        refresh.add_css_class("search-filter-apply");
        refresh.set_tooltip_text(Some(
            "Rescan this scope for added or changed files (Ctrl+R)",
        ));
        scope_bar.append(&refresh);
        panel.append(&scope_bar);

        let category_bar = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        category_bar.add_css_class("search-categories");
        let categories: Vec<_> = SEARCH_GROUPS
            .iter()
            .map(|group| {
                let button = gtk::ToggleButton::with_label(group_label(*group));
                button.add_css_class("search-category");
                button.set_active(*group == SearchGroup::All);
                category_bar.append(&button);
                button
            })
            .collect();
        panel.append(&category_bar);

        let notice = gtk::Label::new(Some(SEARCH_NOTICE_HINT));
        notice.add_css_class("search-notice");
        notice.set_ellipsize(gtk::pango::EllipsizeMode::End);
        notice.set_single_line_mode(true);
        notice.set_hexpand(true);
        notice.set_max_width_chars(1);
        notice.set_xalign(0.0);
        let notice_slot = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        notice_slot.add_css_class("search-notice-slot");
        notice_slot.set_size_request(-1, 32);
        notice_slot.set_valign(gtk::Align::Center);
        notice_slot.append(&interpretation_view);
        notice_slot.append(&notice);
        panel.append(&notice_slot);

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
        let open = gtk::Label::new(Some("↵  open / explore"));
        let filter_hint = gtk::Label::new(Some("Ctrl+F  filters"));
        navigation.add_css_class("search-hint");
        open.add_css_class("search-hint");
        filter_hint.add_css_class("search-hint");
        footer.append(&navigation);
        footer.append(&open);
        footer.append(&filter_hint);
        let more_button = gtk::Button::with_label("Load more");
        more_button.add_css_class("search-filter-clear");
        more_button.set_size_request(64, 32);
        more_button.set_valign(gtk::Align::Center);
        more_button.set_sensitive(false);
        more_button.set_tooltip_text(Some("Load the next page of search results (up to 100)"));
        footer.append(&more_button);
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
            interpretation_slot,
            interpretation_view,
            list,
            results,
            status,
            notice,
            #[cfg(test)]
            notice_slot,
            refresh,
            save_button,
            save_search: RefCell::new(None),
            root: RefCell::new(PathBuf::new()),
            current_folder: RefCell::new(glib::home_dir()),
            scope,
            scope_label,
            selected_key: RefCell::new(None),
            rendering: Cell::new(false),
            visible_results: RefCell::new(Vec::new()),
            visible_commands: RefCell::new(Vec::new()),
            command_handler: RefCell::new(None),
            group: Cell::new(SearchGroup::All),
            entity_scope: RefCell::new(None),
            grouped_headers: Cell::new(false),
            back_button,
            search: RefCell::new(None),
            session_generation: Cell::new(0),
            expected_query_generation: Cell::new(None),
            page: Cell::new(None),
            more_button,
            activate,
            dismiss,
            open_results_button,
            open_results: RefCell::new(None),
            filters,
            categories,
        });

        let headers = Rc::downgrade(&state);
        state.list.set_header_func(move |row, before| {
            let Some(state) = headers.upgrade() else {
                return;
            };
            if !state.visible_commands.borrow().is_empty() {
                results::set_header(row, None);
                return;
            }
            let group = if state.grouped_headers.get() {
                let visible = state.visible_results.borrow();
                let current = usize::try_from(row.index())
                    .ok()
                    .and_then(|position| visible.get(position))
                    .map(SearchResult::group);
                let previous = before
                    .and_then(|row| usize::try_from(row.index()).ok())
                    .and_then(|position| visible.get(position))
                    .map(SearchResult::group);
                current.filter(|group| Some(*group) != previous)
            } else {
                None
            };
            results::set_header(row, group);
        });

        for (position, button) in state.categories.iter().enumerate() {
            let weak_state = Rc::downgrade(&state);
            button.connect_clicked(move |_| {
                if let Some(state) = weak_state.upgrade() {
                    state.entity_scope.replace(None);
                    state.group.set(SEARCH_GROUPS[position]);
                    start_query(&state, &state.field.text());
                    state.field.grab_focus();
                }
            });
        }

        let back = Rc::downgrade(&state);
        state.back_button.connect_clicked(move |_| {
            if let Some(state) = back.upgrade() {
                back_to_results(&state);
            }
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
                let terms = search_filter_terms(&state.field.text());
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
        let more = Rc::downgrade(&state);
        state.more_button.connect_clicked(move |_| {
            if let Some(state) = more.upgrade() {
                request_more(&state);
            }
        });
        let opened = Rc::downgrade(&state);
        state.open_results_button.connect_clicked(move |_| {
            if let Some(state) = opened.upgrade() {
                if !state.open_results_button.is_sensitive()
                    || matches!(
                        state.group.get(),
                        SearchGroup::Albums | SearchGroup::Artists
                    )
                {
                    return;
                }
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
        let saved = Rc::downgrade(&state);
        state.save_button.connect_clicked(move |_| {
            if let Some(state) = saved.upgrade() {
                if !state.save_button.is_sensitive()
                    || matches!(
                        state.group.get(),
                        SearchGroup::Albums | SearchGroup::Artists
                    )
                {
                    return;
                }
                let callback = state.save_search.borrow().clone();
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
                    let key = row
                        .and_then(|row| usize::try_from(row.index()).ok())
                        .and_then(|position| {
                            state
                                .visible_results
                                .borrow()
                                .get(position)
                                .map(SearchResult::key)
                        });
                    state.selected_key.replace(key);
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

    pub fn set_command_handler(&self, handler: Rc<dyn Fn(&str)>) {
        self.state.command_handler.replace(Some(handler));
    }

    pub fn show(&self, root: PathBuf) {
        self.show_query(root, "");
    }

    pub fn show_query(&self, root: PathBuf, query: &str) {
        self.show_scoped(root, query, 1);
    }

    fn show_scoped(&self, root: PathBuf, query: &str, scope: u32) {
        self.state.current_folder.replace(root);
        self.state.search.borrow_mut().take();
        self.state.expected_query_generation.set(None);
        clear_page(&self.state);
        self.state.scope.set_selected(scope);
        if self.state.field.text() != query {
            self.state.field.set_text(query);
        } else {
            begin_query(&self.state, query);
        }
        self.state.field.set_position(-1);
        self.state.layer.set_visible(true);
        restart_search(&self.state);
        self.state.field.grab_focus();
    }

    pub fn show_music(&self) {
        self.show_scoped(SearchScope::MusicLibrary.root(&glib::home_dir()), "", 2);
    }

    pub fn search_uri(&self) -> String {
        let folder = self.state.current_folder.borrow().clone();
        ScopedSearch::new(
            selected_scope(self.state.scope.selected()),
            &folder,
            &self.state.field.text(),
        )
        .with_group(
            self.state.group.get(),
            self.state
                .entity_scope
                .borrow()
                .as_ref()
                .map(|scope| scope.key.clone()),
        )
        .to_uri()
    }

    pub fn set_open_results(&self, callback: Rc<dyn Fn(String)>) {
        self.state.open_results.replace(Some(callback));
        self.state.open_results_button.set_visible(true);
        update_group_controls(&self.state);
    }

    pub fn set_save_search(&self, callback: Rc<dyn Fn(String)>) {
        self.state.save_search.replace(Some(callback));
        let show = crate::ui::theme::ThemeManager::shared().show_smart_folders();
        self.state.save_button.set_visible(show);
        update_group_controls(&self.state);
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

        append_filter_section_heading(&content, "FILES");
        let file_type = gtk::DropDown::from_strings(&SEARCH_TYPES);
        file_type.set_hexpand(true);
        append_filter_control(&content, "Type", &file_type);
        let extension = filter_entry("pdf");
        append_filter_control(&content, "Extension", &extension);
        let within = filter_entry("Documents or Projects/hermes");
        append_filter_control(&content, "Within folder", &within);

        append_filter_section_heading(&content, "MUSIC");
        let artist = filter_entry("Exact artist name");
        append_filter_control(&content, "Artist", &artist);
        let album = filter_entry("Exact album name");
        append_filter_control(&content, "Album", &album);
        let track_title = filter_entry("Exact song title");
        append_filter_control(&content, "Title", &track_title);

        append_filter_section_heading(&content, "CONTENTS");
        let modified = gtk::DropDown::from_strings(&[
            "Any time",
            "Today",
            "Yesterday",
            "Last week",
            "Past 7 days",
        ]);
        modified.set_hexpand(true);
        append_filter_control(&content, "Modified", &modified);
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

        let scroll = gtk::ScrolledWindow::builder()
            .child(&content)
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .propagate_natural_height(true)
            .max_content_height(520)
            .build();
        let popover = gtk::Popover::builder()
            .child(&scroll)
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
            artist,
            album,
            title: track_title,
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
        self.file_type
            .set_selected(if let Some(category) = values.category.as_deref() {
                SEARCH_CATEGORIES
                    .iter()
                    .position(|candidate| *candidate == category)
                    .map_or(0, |index| index as u32 + 3)
            } else {
                match values.directory {
                    Some(false) => 1,
                    Some(true) => 2,
                    None => 0,
                }
            });
        self.modified
            .set_selected(match values.modified.as_deref() {
                Some("today") => 1,
                Some("yesterday") => 2,
                Some("last-week") => 3,
                Some("past-7-days") => 4,
                _ => 0,
            });
        self.extension
            .set_text(values.extension.as_deref().unwrap_or_default());
        self.within
            .set_text(values.within.as_deref().unwrap_or_default());
        self.artist
            .set_text(values.artist.as_deref().unwrap_or_default());
        self.album
            .set_text(values.album.as_deref().unwrap_or_default());
        self.title
            .set_text(values.title.as_deref().unwrap_or_default());
        self.content
            .set_text(values.content.as_deref().unwrap_or_default());
        self.content_enabled.set_active(values.content.is_some());
        self.update_active(values != &SearchFilterValues::default());
    }

    fn query(&self, current: &str) -> String {
        let terms = search_filter_terms(current);
        let mut parts = Vec::new();
        if !terms.is_empty() {
            parts.push(terms);
        }
        match self.file_type.selected() {
            1 => parts.push("type:file".to_owned()),
            2 => parts.push("type:folder".to_owned()),
            position @ 3..=8 => {
                parts.push(format!("type:{}", SEARCH_CATEGORIES[position as usize - 3]))
            }
            _ => {}
        }
        push_filter(&mut parts, "ext", &self.extension.text());
        push_filter(&mut parts, "in", &self.within.text());
        push_filter(&mut parts, "artist", &self.artist.text());
        push_filter(&mut parts, "album", &self.album.text());
        push_filter(&mut parts, "title", &self.title.text());
        match self.modified.selected() {
            1 => parts.push("modified:today".to_owned()),
            2 => parts.push("modified:yesterday".to_owned()),
            3 => parts.push("modified:last-week".to_owned()),
            4 => parts.push("modified:past-7-days".to_owned()),
            _ => {}
        }
        if self.content_enabled.is_active() {
            push_filter(&mut parts, "content", &self.content.text());
        }
        let (_, current_values) = search_filter_values(current);
        if let Some(rating_raw) = current_values.rating_raw {
            parts.push(format!("rating:{rating_raw}"));
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

fn append_filter_section_heading(container: &gtk::Box, title: &str) {
    let heading = gtk::Label::new(Some(title));
    heading.add_css_class("search-filter-section-title");
    heading.set_xalign(0.0);
    container.append(&heading);
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

fn render_inferred_chips(
    state: &Rc<SearchState>,
    inferred: &[InferredFilterChip],
) {
    state.interpretation_slot.set_visible(!inferred.is_empty());
    state.interpretation_view.set_visible(!inferred.is_empty());
    while let Some(child) = state.interpretation_slot.first_child() {
        state.interpretation_slot.remove(&child);
    }
    for chip in inferred {
        let pill = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        pill.add_css_class("search-inferred-chip");
        pill.set_valign(gtk::Align::Center);
        pill.set_hexpand(false);
        pill.set_tooltip_text(Some(&chip.label));
        let label = gtk::Label::new(Some(&chip.label));
        label.add_css_class("search-inferred-chip-label");
        label.set_ellipsize(gtk::pango::EllipsizeMode::End);
        label.set_single_line_mode(true);
        label.set_max_width_chars(18);
        label.set_hexpand(true);
        pill.append(&label);
        let remove = gtk::Button::builder()
            .tooltip_text(format!("Dismiss filter and search for \"{}\"", &chip.token_text))
            .build();
        remove.add_css_class("search-inferred-chip-remove");
        remove.set_child(Some(&crate::assets::primary_icon(
            crate::assets::icons::X,
            10,
        )));
        let weak_state = Rc::downgrade(state);
        let chip = chip.clone();
        remove.connect_clicked(move |_| {
            if let Some(state) = weak_state.upgrade() {
                dismiss_inferred_chip(&state, &chip);
            }
        });
        pill.append(&remove);
        state.interpretation_slot.append(&pill);
    }
}

fn dismiss_inferred_chip(state: &Rc<SearchState>, chip: &InferredFilterChip) {
    let current = state.field.text().to_string();
    let Some(new_query) = chip.literal_query(&current) else {
        return;
    };
    state.field.set_text(&new_query);
    state.field.set_position(-1);
    state.field.grab_focus();
}

fn push_filter(parts: &mut Vec<String>, name: &str, value: &str) {
    let value = value.trim();
    if value.is_empty() {
        return;
    }
    let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
    parts.push(format!("{name}:\"{escaped}\""));
}

fn group_label(group: SearchGroup) -> &'static str {
    match group {
        SearchGroup::All => "All",
        SearchGroup::Files => "Files",
        SearchGroup::Songs => "Songs",
        SearchGroup::Albums => "Albums",
        SearchGroup::Artists => "Artists",
    }
}

fn update_group_controls(state: &SearchState) {
    for (group, button) in SEARCH_GROUPS.iter().zip(&state.categories) {
        button.set_active(*group == state.group.get());
    }
    let entity = state
        .entity_scope
        .borrow()
        .as_ref()
        .map(|scope| scope.key.clone());
    state.back_button.set_sensitive(entity.is_some());
    let description = entity.as_ref().map(results::entity_description);
    let root = state.root.borrow().to_string_lossy().into_owned();
    let scope = description.map_or(root.clone(), |description| {
        format!("{root} · {description}")
    });
    state.scope_label.set_text(&scope);
    state.scope_label.set_tooltip_text(Some(&scope));
    let exportable = !matches!(
        state.group.get(),
        SearchGroup::Albums | SearchGroup::Artists
    );
    state.open_results_button.set_sensitive(exportable);
    let show_save = state.save_search.borrow().is_some() && crate::ui::theme::ThemeManager::shared().show_smart_folders();
    state.save_button.set_visible(show_save);
    state
        .save_button
        .set_sensitive(exportable && state.save_search.borrow().is_some());
    state.save_button.set_tooltip_text(Some(if exportable {
        "Keep this scope and query as a Smart Folder; files stay where they are"
    } else {
        "Open an album or artist first to save its matching songs"
    }));
    state
        .open_results_button
        .set_tooltip_text(Some(if exportable {
            "Show these file or song matches in a tab"
        } else {
            "Open an album or artist first to show its matching songs in a tab"
        }));
}

fn clear_entity_scope(state: &SearchState) {
    let scope = state.entity_scope.borrow_mut().take();
    if let Some(scope) = scope {
        state.group.set(scope.previous_group);
    }
}

fn begin_query(state: &Rc<SearchState>, query: &str) {
    clear_entity_scope(state);
    start_query(state, query);
}

fn start_query(state: &Rc<SearchState>, query: &str) {
    state.expected_query_generation.set(None);
    clear_page(state);

    if let Some(cmd_query) = query.strip_prefix('>') {
        let commands = matching_commands(cmd_query);
        state.visible_commands.replace(commands.clone());
        state.visible_results.borrow_mut().clear();
        state.selected_key.replace(None);
        state.filters.set_values(&SearchFilterValues::default());
        render_inferred_chips(state, &[]);
        update_group_controls(state);

        state.rendering.set(true);
        while let Some(row) = state.list.row_at_index(0) {
            state.list.remove(&row);
        }
        for cmd in &commands {
            let row = build_command_row(cmd);
            state.list.append(&row);
        }
        state.rendering.set(false);

        state.notice.set_text(if commands.is_empty() {
            "No matching commands"
        } else {
            "Command Palette · Press Enter to run"
        });
        state.notice.set_tooltip_text(None);

        if commands.is_empty() {
            state.status.set_text(
                "No matching commands found.\nTry '>settings', '>view', '>music', '>home', or '>tab'.",
            );
            state.results.set_visible_child_name("status");
        } else {
            state.results.set_visible_child_name("list");
            if let Some(first) = state.list.row_at_index(0) {
                state.list.select_row(Some(&first));
            }
        }
        return;
    }

    state.visible_commands.borrow_mut().clear();

    let query = if let Some(stripped) = query.strip_prefix("home:") {
        state.scope.set_selected(1);
        stripped.trim_start()
    } else if let Some(stripped) = query.strip_prefix("folder:") {
        state.scope.set_selected(0);
        stripped.trim_start()
    } else if let Some(stripped) = query.strip_prefix("music:") {
        state.scope.set_selected(2);
        stripped.trim_start()
    } else {
        query
    };

    let (_, filters) = search_filter_values(query);
    state.filters.set_values(&filters);
    render_inferred_chips(state, &filters.inferred);
    update_group_controls(state);
    state.selected_key.replace(None);
    state.visible_results.borrow_mut().clear();
    state.rendering.set(true);
    while let Some(row) = state.list.row_at_index(0) {
        state.list.remove(&row);
    }
    state.rendering.set(false);
    state.notice.set_text(if query.trim().is_empty() {
        SEARCH_NOTICE_HINT
    } else {
        "Searching…"
    });
    state.notice.set_tooltip_text(None);
    state.results.set_visible_child_name("status");
    if query.trim().is_empty() {
        state.status.set_text(SEARCH_HELP);
    } else {
        state.status.set_text("Searching…");
    }
    if let Some(search) = state.search.borrow().as_ref() {
        let entity = state
            .entity_scope
            .borrow()
            .as_ref()
            .map(|scope| scope.key.clone());
        state.expected_query_generation.set(Some(search.query_group(
            query,
            state.group.get(),
            entity,
        )));
    }
}

#[cfg(test)]
fn render_results(
    state: &Rc<SearchState>,
    results: Vec<SearchItem>,
    indexing: bool,
    notice: Option<&str>,
) {
    state.grouped_headers.set(false);
    render_search_results(
        state,
        results.into_iter().map(SearchResult::File).collect(),
        indexing,
        notice,
        None,
    );
}

fn render_search_results(
    state: &Rc<SearchState>,
    results: Vec<SearchResult>,
    indexing: bool,
    notice: Option<&str>,
    counts: Option<SearchCounts>,
) {
    let selected_key = state.selected_key.borrow().clone();
    let awaiting_selection = indexing
        && selected_key
            .as_ref()
            .is_some_and(|key| !results.iter().any(|item| &item.key() == key));
    let selected = selection_position(&results, selected_key.as_ref());
    let root = state.root.borrow().clone();
    let changed = *state.visible_results.borrow() != results;
    let existing_rows: Vec<_> = if changed {
        (0..state.visible_results.borrow().len())
            .filter_map(|position| state.list.row_at_index(position as i32))
            .collect()
    } else {
        Vec::new()
    };
    let rows: Vec<_> = if changed {
        let mut previous: HashMap<_, _> = state
            .visible_results
            .borrow()
            .iter()
            .enumerate()
            .filter_map(|(position, item)| {
                Some((
                    item.key(),
                    (item.clone(), state.list.row_at_index(position as i32)?),
                ))
            })
            .collect();
        results
            .iter()
            .map(|item| {
                previous
                    .remove(&item.key())
                    .filter(|(previous, _)| previous == item)
                    .map(|(_, row)| row)
                    .unwrap_or_else(|| results::result_row(item, &root))
            })
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
    let count = state
        .page
        .get()
        .map(|page| format!("{} of {} matches", page.shown, page.total))
        .unwrap_or_default();
    let groups = counts
        .map(|counts| {
            format!(
                "{} files · {} songs · {} albums · {} artists",
                counts.files, counts.songs, counts.albums, counts.artists
            )
        })
        .unwrap_or_default();
    let notice = [
        count.as_str(),
        groups.as_str(),
        progress,
        notice.unwrap_or_default(),
    ]
    .into_iter()
    .filter(|text| !text.is_empty())
    .collect::<Vec<_>>()
    .join(" · ");
    let has_notice = !notice.is_empty();
    state.notice.set_text(if has_notice {
        &notice
    } else {
        SEARCH_NOTICE_HINT
    });
    state
        .notice
        .set_tooltip_text(has_notice.then_some(notice.as_str()));
    if changed {
        let retained: HashSet<_> = rows.iter().cloned().collect();
        for current in existing_rows {
            if !retained.contains(&current) {
                state.list.remove(&current);
            }
        }
        for (position, row) in rows.iter().enumerate() {
            if state.list.row_at_index(position as i32).as_ref() == Some(row) {
                continue;
            }
            if row.parent().is_some() {
                state.list.remove(row);
            }
            state.list.insert(row, position as i32);
        }
    }
    state.list.invalidate_headers();
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
                "No matching files, songs, albums or artists in this scope"
            });
    }
    state.rendering.set(false);
    let selected_key = selected.and_then(|position| {
        state
            .visible_results
            .borrow()
            .get(position)
            .map(SearchResult::key)
    });
    if !awaiting_selection {
        state.selected_key.replace(selected_key);
    }
}

fn move_selection(state: &SearchState, direction: i32) {
    let count = if !state.visible_commands.borrow().is_empty() {
        state.visible_commands.borrow().len() as i32
    } else {
        state.visible_results.borrow().len() as i32
    };
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
    if !state.visible_commands.borrow().is_empty() {
        if let Some(cmd) = usize::try_from(position)
            .ok()
            .and_then(|idx| state.visible_commands.borrow().get(idx).cloned())
        {
            hide(state);
            if let Some(handler) = state.command_handler.borrow().as_ref() {
                handler(cmd.id);
            }
        }
        return;
    }
    let Some(item) = usize::try_from(position)
        .ok()
        .and_then(|position| state.visible_results.borrow().get(position).cloned())
    else {
        return;
    };
    match item {
        SearchResult::File(item) => {
            hide(state);
            (state.activate)(item);
        }
        SearchResult::Album(entity) | SearchResult::Artist(entity) => {
            let previous_selection = Some(SearchResultKey::Entity(entity.key.clone()));
            state.entity_scope.replace(Some(EntityScope {
                key: entity.key,
                previous_group: state.group.get(),
                previous_selection,
            }));
            state.group.set(SearchGroup::Songs);
            start_query(state, &state.field.text());
            state.field.grab_focus();
        }
    }
}

fn back_to_results(state: &Rc<SearchState>) {
    let scope = state.entity_scope.borrow_mut().take();
    let Some(scope) = scope else {
        return;
    };
    state.group.set(scope.previous_group);
    start_query(state, &state.field.text());
    state.selected_key.replace(scope.previous_selection);
    state.field.grab_focus();
}

fn hide(state: &SearchState) {
    state
        .session_generation
        .set(state.session_generation.get().wrapping_add(1));
    state.expected_query_generation.set(None);
    clear_page(state);
    clear_entity_scope(state);
    update_group_controls(state);
    state.search.borrow_mut().take();
    state.layer.set_visible(false);
    (state.dismiss)();
}

fn selected_scope(position: u32) -> SearchScope {
    match position {
        0 => SearchScope::ThisFolder,
        2 => SearchScope::MusicLibrary,
        _ => SearchScope::Home,
    }
}

fn selection_position(
    results: &[SearchResult],
    selected: Option<&SearchResultKey>,
) -> Option<usize> {
    selected
        .and_then(|selected| results.iter().position(|item| &item.key() == selected))
        .or_else(|| (!results.is_empty()).then_some(0))
}

fn restart_search(state: &Rc<SearchState>) {
    state
        .session_generation
        .set(state.session_generation.get().wrapping_add(1));
    let session_generation = state.session_generation.get();
    state.expected_query_generation.set(None);
    clear_page(state);
    state.notice.set_text("Searching…");
    state.notice.set_tooltip_text(None);
    state.search.borrow_mut().take();
    let had_entity = state.entity_scope.borrow().is_some();
    clear_entity_scope(state);
    let folder = state.current_folder.borrow().clone();
    let root = selected_scope(state.scope.selected()).root(&folder);
    if *state.root.borrow() != root || had_entity {
        state.selected_key.replace(None);
        state.visible_results.borrow_mut().clear();
        state.rendering.set(true);
        while let Some(row) = state.list.row_at_index(0) {
            state.list.remove(&row);
        }
        state.rendering.set(false);
        state.results.set_visible_child_name("status");
        state.status.set_text("Searching selected scope…");
    }
    state.root.replace(root.clone());
    update_group_controls(state);
    let (handle, receiver) = index_tree(root, Arc::new(LocalTextExtractionProvider));
    state.expected_query_generation.set(Some(handle.query_group(
        &state.field.text(),
        state.group.get(),
        None,
    )));
    state.search.replace(Some(handle));
    let weak = Rc::downgrade(state);
    let _poll = glib::timeout_add_local(Duration::from_millis(16), move || {
        let Some(state) = weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        if !state.layer.is_visible() || state.session_generation.get() != session_generation {
            return glib::ControlFlow::Break;
        }
        if let Some(event) = receiver.try_recv() {
            apply_search_event(&state, session_generation, event);
        }
        glib::ControlFlow::Continue
    });
}

fn is_current_search_response(
    session_generation: u64,
    response_session_generation: u64,
    expected_query_generation: Option<u64>,
    response_query_generation: u64,
) -> bool {
    session_generation == response_session_generation
        && expected_query_generation == Some(response_query_generation)
}

fn apply_search_event(state: &Rc<SearchState>, session_generation: u64, event: SearchEvent) {
    let SearchEvent::Results {
        generation,
        items,
        indexing,
        notice,
        page,
        groups,
        ..
    } = event;
    if state.layer.is_visible()
        && is_current_search_response(
            state.session_generation.get(),
            session_generation,
            state.expected_query_generation.get(),
            generation,
        )
        && !page.is_some_and(|page| {
            page.generation != generation
                || state.page.get().is_some_and(|current| {
                    current.generation == page.generation
                        && (current.revision > page.revision
                            || (current.revision == page.revision && current.shown > page.shown))
                })
        })
        && groups
            .as_ref()
            .is_none_or(|groups| groups.group == state.group.get())
    {
        state.page.set(page);
        state
            .more_button
            .set_sensitive(page.is_some_and(|page| page.has_more()));
        state.grouped_headers.set(
            groups
                .as_ref()
                .is_some_and(|groups| groups.group == SearchGroup::All),
        );
        let (results, counts) = match groups {
            Some(SearchGroups {
                results, counts, ..
            }) => (results, Some(counts)),
            None => (items.into_iter().map(SearchResult::File).collect(), None),
        };
        render_search_results(state, results, indexing, notice.as_deref(), counts);
    }
}

fn clear_page(state: &SearchState) {
    state.page.set(None);
    state.more_button.set_sensitive(false);
}

fn request_more(state: &SearchState) {
    if !state.layer.is_visible() || !state.more_button.is_sensitive() {
        return;
    }
    let Some(page) = state.page.get().filter(|page| {
        page.has_more() && state.expected_query_generation.get() == Some(page.generation)
    }) else {
        return;
    };
    if let Some(search) = state.search.borrow().as_ref() {
        state.more_button.set_sensitive(false);
        search.more(page);
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod saved_tests;
