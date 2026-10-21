// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    cell::{Cell, RefCell},
    env,
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};

use gtk::{gio, glib, prelude::*};

use crate::{
    adapters::{LocalOperationProvider, LocalPreviewProvider, LocalTrailStore, RoutedFileSource},
    app::{Browser, BrowserEvent, Trails},
    model::{
        Location, Trail, TrailBrowserDensity,
        TrailBrowserMode, TrailViewState,
    },
};

use super::{
    blur::BlurBin,
    browser::{BrowserView, PeekBehavior},
    browser_modes::{BrowserDensity, BrowserMode},
    motion::{animations_enabled, emphasized_deceleration},
    preview::PreviewDrawer,
    search::SearchDialog,
    theme::ThemeManager,
    trails::TabBar,
};

const SIDEBAR_WIDTH: i32 = 208;
const MIN_SIDEBAR_WIDTH: i32 = 176;
const SIDEBAR_TRANSITION: Duration = Duration::from_millis(300);

mod saved_searches;
mod startup;

pub fn present(application: &gtk::Application) {
    present_location(application, None);
}

pub fn present_location(application: &gtk::Application, location: Option<PathBuf>) {
    crate::assets::register_icon_theme();
    let theme_manager = super::theme::ThemeManager::shared();
    let trail_store = Rc::new(LocalTrailStore::xdg());
    let trails = Trails::load(trail_store.clone()).unwrap_or_else(|error| {
        tracing::warn!(%error, "unable to load saved Trails");
        Trails::empty(trail_store.clone())
    });
    let home = Location::local(home_directory());
    let fresh_view = startup::fresh_view(
        trail_browser_mode(theme_manager.browser_mode()),
        trail_browser_density(theme_manager.browser_density()),
        theme_manager.show_hidden_default(),
    );
    let startup = startup::select(
        &trails,
        location.map(Location::local),
        &theme_manager.startup_location(),
        &home,
        &fresh_view,
    );
    let startup_restore = Rc::new(RefCell::new(startup::Restore::new(&startup.trail)));
    let applying_startup = Rc::new(Cell::new(false));
    load_styles();

    let window = gtk::ApplicationWindow::builder()
        .application(application)
        .title("Strata")
        .default_width(1200)
        .default_height(760)
        .build();

    let browser = BrowserView::new(Rc::new(RoutedFileSource::new()), PeekBehavior::default());
    let initial_view = startup.trail.view.clone();
    browser.set_view_mode(browser_mode(initial_view.browser_mode));
    browser.set_density(browser_density(initial_view.density));
    browser.set_operation_provider(Rc::new(LocalOperationProvider));
    let controller = browser.browser();
    controller.set_preferences(initial_view.preferences);
    let preview = PreviewDrawer::new(Rc::new(LocalPreviewProvider));
    preview.set_image_context_menu_handler(browser.image_context_menu_handler());
    let music_player = super::music::MusicPlayer::for_application(application);
    install_audio_playback_coordination(&window, &preview, music_player.controller());
    let activated_music = music_player.clone();
    let activated_music_preview = preview.clone();
    browser.set_open_handler(Rc::new(move |location| {
        let Some(path) = location.native_path() else { return false; };
        if crate::services::classify_by_name(path.as_os_str()) != crate::services::FormatFamily::Audio {
            return false;
        }
        activated_music_preview.close();
        activated_music.play_folder(path.to_owned());
        true
    }));
    let preview_for_selection = preview.clone();
    let weak_controller = Rc::downgrade(&controller);
    let observed_trails = trails.clone();
    let observed_startup = startup_restore.clone();
    let observed_applying_startup = applying_startup.clone();
    controller.observe(move |event| match event {
        BrowserEvent::ColumnAdded { .. } | BrowserEvent::ColumnsTruncated { .. }
        | BrowserEvent::TabRestored { .. } => {
            let Some(browser) = weak_controller.upgrade() else {
                return;
            };
            let Some(active_location) = browser.active_location() else {
                return;
            };
            if observed_applying_startup.get()
                || observed_startup.borrow_mut().preserves(
                    observed_trails.active_id().as_ref(),
                    Some(&active_location),
                )
            {
                return;
            }
            if let Err(error) = observed_trails.update_active_location(active_location) {
                tracing::warn!(%error, "unable to save active Trail");
            }
        }
        BrowserEvent::PreviewRequested { entry } => preview_for_selection.show(entry),
        BrowserEvent::FocusChanged {
            depth,
            position: Some(position),
        } if preview_for_selection.is_open() => {
            if let Some(entry) = weak_controller
                .upgrade()
                .and_then(|browser| browser.entry_at(depth, position))
            {
                if entry.is_directory() {
                    preview_for_selection.close();
                } else {
                    preview_for_selection.show(entry);
                }
            }
        }
        _ => {}
    });

    let header = gtk::HeaderBar::new();
    header.set_show_title_buttons(false);
    header.set_title_widget(Some(&gtk::Box::new(gtk::Orientation::Horizontal, 0)));
    let music_toggle = gtk::ToggleButton::builder()
        .tooltip_text("Music player · Ctrl+M")
        .build();
    music_toggle.set_child(Some(&crate::assets::primary_icon(
        crate::assets::icons::MUSIC,
        18,
    )));
    music_toggle.add_css_class("header-action");
    let sidebar_toggle = gtk::ToggleButton::builder()
        .active(initial_view.sidebar_open)
        .tooltip_text("Toggle sidebar")
        .build();
    sidebar_toggle.set_child(Some(&crate::assets::primary_icon(
        crate::assets::icons::PANEL_LEFT,
        20,
    )));
    sidebar_toggle.add_css_class("sidebar-toggle");
    let captured_browser = browser.clone();
    let captured_controller = controller.clone();
    let captured_sidebar = sidebar_toggle.clone();
    let captured_preview = preview.clone();
    let capture_view: Rc<dyn Fn() -> TrailViewState> = Rc::new(move || TrailViewState {
        browser_mode: trail_browser_mode(captured_browser.view_mode()),
        density: trail_browser_density(captured_browser.density()),
        preferences: captured_controller
            .active_depth()
            .and_then(|depth| captured_controller.column_preferences(depth))
            .unwrap_or_default(),
        sidebar_open: captured_sidebar.is_active(),
        preview_open: captured_preview.is_open(),
    });
    let mode_action = gio::SimpleAction::new_stateful(
        "view-mode",
        Some(glib::VariantTy::STRING),
        &browser.view_mode().action_name().to_variant(),
    );
    let mode_browser = browser.clone();
    let mode_preferences = theme_manager.clone();
    let mode_trails = trails.clone();
    let mode_capture = capture_view.clone();
    mode_action.connect_activate(move |action, parameter| {
        let Some(mode) = parameter
            .and_then(|parameter| parameter.str())
            .and_then(BrowserMode::from_action_name)
        else {
            return;
        };
        mode_browser.set_view_mode(mode);
        mode_preferences.set_browser_mode(mode);
        action.set_state(&mode.action_name().to_variant());
        if let Err(error) = mode_trails.update_active_view(mode_capture()) {
            tracing::warn!(%error, "unable to save Trail view mode");
        }
    });
    window.add_action(&mode_action);
    let activated_browser = browser.clone();
    let activated_sidebar = sidebar_toggle.clone();
    let activated_preview = preview.clone();
    let file_workspace_toggle = music_toggle.clone();
    let activated_trails = trails.clone();
    let activated_view = capture_view.clone();
    let activate_tab: Rc<dyn Fn(Trail)> = Rc::new(move |trail| {
        file_workspace_toggle.set_active(false);
        activated_preview.close();
        activated_browser.activate_tab(&trail);
        activated_sidebar.set_active(trail.view.sidebar_open);
        if let Err(error) = activated_trails.update_active_view(activated_view()) {
            tracing::warn!(%error, "unable to save activated tab view");
        }
    });
    let tab_bar = TabBar::new(
        trails.clone(),
        controller.clone(),
        capture_view.clone(),
        activate_tab.clone(),
    );
    let context_trails = trails.clone();
    let context_view = capture_view.clone();
    let context_tab_bar = tab_bar.clone();
    let status_bar = Rc::new(super::status_bar::StatusBar::new());
    if let Some(path) = startup.trail.active_location().and_then(Location::native_path) {
        status_bar.update_free_space(path);
    }
    let context_status_bar = status_bar.clone();
    let context_controller = controller.clone();
    let context_startup = startup_restore.clone();
    let context_applying_startup = applying_startup.clone();
    controller.observe(move |event| {
        if matches!(
            event,
            BrowserEvent::ColumnAdded { .. }
                | BrowserEvent::TabRestored { .. }
                | BrowserEvent::ColumnsTruncated { .. }
                | BrowserEvent::SortingFinished { .. }
                | BrowserEvent::PreviewRequested { .. }
                | BrowserEvent::FocusChanged { .. }
        ) {
            if !context_applying_startup.get()
                && !context_startup.borrow_mut().preserves(
                    context_trails.active_id().as_ref(),
                    context_controller.active_location().as_ref(),
                )
            {
                if let Err(error) = context_trails.update_active_view(context_view()) {
                    tracing::warn!(%error, "unable to save Trail view state");
                }
            }
            context_tab_bar.refresh();
        }
        if matches!(
            event,
            BrowserEvent::EntriesInserted { .. }
                | BrowserEvent::TabRestored { .. }
                | BrowserEvent::EntriesReplaced { .. }
                | BrowserEvent::EntriesSpliced { .. }
                | BrowserEvent::SelectionSetChanged { .. }
                | BrowserEvent::FocusChanged { .. }
                | BrowserEvent::ColumnAdded { .. }
                | BrowserEvent::ColumnsTruncated { .. }
                | BrowserEvent::ColumnReloaded { .. }
                | BrowserEvent::Reset
        ) {
            let mut total_items = 0;
            let mut selected_items = 0;
            let mut selected_bytes = 0;
            
            if let Some(depth) = context_controller.active_depth() {
                if let Some(entries) = context_controller.column_entries(depth) {
                    total_items = entries.len();
                }
                let selected = context_controller.selected_entries();
                selected_items = selected.len();
                for entry in selected {
                    if let crate::model::MetadataValue::Known(size) = entry.size {
                        selected_bytes += size;
                    }
                }
            }
            context_status_bar.update_item_count(total_items);
            context_status_bar.update_selection(selected_items, selected_bytes);

            if !matches!(event, BrowserEvent::SelectionSetChanged { .. }) {
                if let Some(path) = context_controller
                    .active_location()
                    .as_ref()
                    .and_then(|l| l.native_path())
                {
                    context_status_bar.update_free_space(path);
                } else {
                    context_status_bar.clear_free_space();
                }
            }
        }
    });
    header.pack_start(&sidebar_toggle);
    header.pack_start(&music_toggle);
    header.pack_start(&browser.location_widget());
    let search_button = gtk::Button::builder()
        .tooltip_text("Search (Ctrl+K)")
        .build();
    let search_icon = crate::assets::text_icon(crate::assets::icons::SEARCH, 16);
    search_icon.set_halign(gtk::Align::End);
    search_icon.set_hexpand(true);
    search_button.set_child(Some(&search_icon));
    search_button.add_css_class("header-action");
    search_button.add_css_class("search-action");
    let appearance = build_appearance_menu(
        &browser,
        &controller,
        theme_manager.clone(),
        trails.clone(),
        capture_view.clone(),
    );
    let settings = gtk::Button::builder().tooltip_text("Settings").build();
    settings.set_child(Some(&crate::assets::text_icon(
        crate::assets::icons::SETTINGS,
        20,
    )));
    settings.add_css_class("header-action");
    let close_window = gtk::Button::builder().tooltip_text("Close window").build();
    close_window.set_child(Some(&crate::assets::text_icon(crate::assets::icons::X, 20)));
    close_window.add_css_class("header-action");
    let closing_window = window.clone();
    close_window.connect_clicked(move |_| closing_window.close());
    let header_actions = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    header_actions.add_css_class("header-actions");
    header_actions.append(&search_button);
    header_actions.append(&tab_bar.new_button());
    header_actions.append(&appearance);
    header_actions.append(&settings);
    header_actions.append(&close_window);
    header.pack_end(&header_actions);

    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.append(&header);
    root.append(&tab_bar.widget());

    let content = gtk::Paned::new(gtk::Orientation::Horizontal);
    content.set_wide_handle(false);
    content.set_shrink_start_child(false);
    content.set_resize_start_child(false);
    content.set_position(SIDEBAR_WIDTH);
    content.set_vexpand(true);
    let sidebar = build_sidebar(browser.clone(), theme_manager.clone());
    let weak_sidebar = Rc::downgrade(&sidebar.state);
    browser.set_pin_handler(Rc::new(move |location, name| {
        if let Some(sidebar) = weak_sidebar.upgrade() {
            sidebar.pin_location(location, name);
        }
    }));
    sidebar.widget.set_size_request(MIN_SIDEBAR_WIDTH, -1);
    content.set_start_child(Some(&sidebar.widget));

    let tree_paned = gtk::Paned::new(gtk::Orientation::Horizontal);
    tree_paned.set_wide_handle(false);
    tree_paned.set_shrink_start_child(false);
    tree_paned.set_resize_start_child(false);
    tree_paned.set_position(220);

    let folder_tree = super::folder_tree::build_folder_tree(browser.clone());
    folder_tree.set_visible(theme_manager.show_folder_tree());

    tree_paned.set_start_child(Some(&folder_tree));
    let browser_vbox = gtk::Box::new(gtk::Orientation::Vertical, 0);
    browser_vbox.append(&browser.widget());
    browser_vbox.append(&status_bar.container);
    
    tree_paned.set_end_child(Some(&browser_vbox));

    content.set_end_child(Some(&tree_paned));
    let animation_generation = Rc::new(Cell::new(0));
    let sidebar_animating = Rc::new(Cell::new(false));
    let constrained_content = content.clone();
    let constrained_toggle = sidebar_toggle.clone();
    let constrained_animation = sidebar_animating.clone();
    content.connect_position_notify(move |_| {
        if constrained_toggle.is_active()
            && !constrained_animation.get()
            && constrained_content.position() < MIN_SIDEBAR_WIDTH
        {
            constrained_content.set_position(MIN_SIDEBAR_WIDTH);
        }
    });
    let animated_content = content.clone();
    let animated_sidebar = sidebar.widget.clone();
    let sidebar_trails = trails.clone();
    let sidebar_view = capture_view.clone();
    let sidebar_applying_startup = applying_startup.clone();
    sidebar_toggle.connect_toggled(move |toggle| {
        animate_sidebar(
            &animated_content,
            &animated_sidebar,
            &animation_generation,
            &sidebar_animating,
            toggle.is_active(),
        );
        if !sidebar_applying_startup.get() {
            if let Err(error) = sidebar_trails.update_active_view(sidebar_view()) {
                tracing::warn!(%error, "unable to save Trail sidebar state");
            }
        }
    });
    let preview_split = gtk::Paned::new(gtk::Orientation::Horizontal);
    preview_split.add_css_class("preview-split");
    preview_split.set_wide_handle(false);
    preview_split.set_resize_start_child(true);
    preview_split.set_resize_end_child(false);
    preview_split.set_shrink_start_child(false);
    preview_split.set_shrink_end_child(true);
    preview_split.set_start_child(Some(&content));
    preview_split.set_end_child(Some(&preview.widget()));
    preview_split.set_position(i32::MAX);
    preview_split.set_vexpand(true);
    let measured_content = content.clone();
    let measured_browser = browser.clone();
    let measured_split = preview_split.downgrade();
    preview.attach_split(
        &preview_split,
        Rc::new(move || {
            let sidebar_width = measured_content.position();
            let available = measured_split.upgrade().map_or(0, |split| split.width());
            sidebar_width
                + measured_browser.preview_occupied_width(available.saturating_sub(sidebar_width))
        }),
    );
    let workspace = gtk::Stack::new();
    workspace.set_vexpand(true);
    workspace.add_named(&preview_split, Some("files"));
    workspace.add_named(&music_player.view(), Some("music"));

    let ws_for_imm_close = workspace.downgrade();
    let ws_for_imm_lib = workspace.downgrade();
    let toggle_for_imm_close = music_toggle.downgrade();
    let toggle_for_imm_lib = music_toggle.downgrade();
    let music_for_imm_close = music_player.downgrade();
    let music_for_imm_lib = music_player.downgrade();
    let immersive_view = super::music::immersive::ImmersiveView::new(
        music_player.controller(),
        Rc::new(move || {
            if let (Some(toggle), Some(workspace), Some(music)) = (
                toggle_for_imm_close.upgrade(),
                ws_for_imm_close.upgrade(),
                music_for_imm_close.upgrade(),
            ) {
                toggle.set_active(true);
                workspace.set_visible_child_name("music");
                music.set_workspace("music");
            }
        }),
        Rc::new(move || {
            if let (Some(toggle), Some(workspace), Some(music)) = (
                toggle_for_imm_lib.upgrade(),
                ws_for_imm_lib.upgrade(),
                music_for_imm_lib.upgrade(),
            ) {
                toggle.set_active(true);
                workspace.set_visible_child_name("music");
                music.set_workspace("music");
            }
        }),
    );
    workspace.add_named(immersive_view.widget(), Some("immersive"));
    workspace.set_visible_child_name("files");
    root.append(&workspace);
    root.append(&music_player.bar());

    let ws_for_imm_open = workspace.downgrade();
    let toggle_for_imm_open = music_toggle.downgrade();
    let imm_for_open = immersive_view.clone();
    let music_for_imm_open = music_player.downgrade();
    music_player.connect_open_immersive(Rc::new(move || {
        if let (Some(workspace), Some(toggle), Some(music)) = (
            ws_for_imm_open.upgrade(),
            toggle_for_imm_open.upgrade(),
            music_for_imm_open.upgrade(),
        ) {
            workspace.set_visible_child_name("immersive");
            music.set_workspace("immersive");
            toggle.set_active(false);
            imm_for_open.sync();
        }
    }));

    let shown_workspace = workspace.clone();
    let music_for_toggle = music_player.clone();
    music_toggle.connect_toggled(move |button| {
        if shown_workspace.visible_child_name().as_deref() == Some("immersive") && !button.is_active() {
            return;
        }
        let target = if button.is_active() { "music" } else { "files" };
        shown_workspace.set_visible_child_name(target);
        music_for_toggle.set_workspace(target);
    });
    let artwork_music_toggle = music_toggle.downgrade();
    music_player.connect_open_player(Rc::new(move || {
        if let Some(toggle) = artwork_music_toggle.upgrade() {
            toggle.set_active(true);
        }
    }));
    let music_action = gio::SimpleAction::new("music", None);
    let toggled_music = music_toggle.clone();
    music_action.connect_activate(move |_, _| toggled_music.set_active(!toggled_music.is_active()));
    window.add_action(&music_action);
    application.set_accels_for_action("win.music", &["<Control>m"]);

    let immersive_action = gio::SimpleAction::new("immersive", None);
    let ws_for_imm_action = workspace.clone();
    let toggle_for_imm_action = music_toggle.clone();
    let imm_for_action = immersive_view.clone();
    let music_for_imm_action = music_player.clone();
    immersive_action.connect_activate(move |_, _| {
        if ws_for_imm_action.visible_child_name().as_deref() == Some("immersive") {
            toggle_for_imm_action.set_active(true);
            ws_for_imm_action.set_visible_child_name("music");
            music_for_imm_action.set_workspace("music");
        } else {
            ws_for_imm_action.set_visible_child_name("immersive");
            music_for_imm_action.set_workspace("immersive");
            toggle_for_imm_action.set_active(false);
            imm_for_action.sync();
        }
    });
    window.add_action(&immersive_action);
    application.set_accels_for_action("win.immersive", &["<Control>i"]);

    let mini_player_action = gio::SimpleAction::new("mini-player", None);
    let music_for_mini = music_player.clone();
    mini_player_action.connect_activate(move |_, _| {
        music_for_mini.toggle_mini_player();
    });
    window.add_action(&mini_player_action);
    application.set_accels_for_action("win.mini-player", &["<Control><Shift>m"]);
    let play_music = gio::SimpleAction::new("play-music-selection", None);
    let selected_music = music_player.clone();
    let selected_controller = controller.clone();
    let selected_preview = preview.clone();
    let selected_music_toggle = music_toggle.clone();
    play_music.connect_activate(move |_, _| {
        let entries = selected_controller.selected_entries();
        if entries.iter().any(crate::app::music_order::is_audio) {
            selected_preview.close();
            selected_music.play_files(entries);
        } else if entries.len() == 1 && entries[0].is_directory() {
            if let Some(path) = entries[0].location.native_path() {
                selected_preview.close();
                selected_music.play_folder(path.to_path_buf());
                selected_music_toggle.set_active(true);
            }
        }
    });
    window.add_action(&play_music);
    application.set_accels_for_action("win.play-music-selection", &["<Control>Return"]);
    let queue_music = gio::SimpleAction::new("queue-music-selection", None);
    let queued_music = music_player.clone();
    let queued_controller = controller.clone();
    queue_music.connect_activate(move |_, _| queued_music.queue_files(queued_controller.selected_entries()));
    window.add_action(&queue_music);

    let window_overlay = gtk::Overlay::new();
    let blurred_root = BlurBin::new(&root);
    window_overlay.set_child(Some(&blurred_root));

    let search_controller = controller.clone();
    let search_preview = preview.clone();
    let search_preferences = theme_manager.clone();
    let activate_search_result = Rc::new(move |item: crate::services::SearchItem| {
        let location = Location::local(item.path.clone());
        if item.is_directory {
            search_preview.close();
            search_controller.navigate(location);
            return;
        }
        search_controller.reveal(location.clone());
        if search_preferences.search_open_files_directly() {
            search_controller.open_location(location);
        } else {
            search_preview.show(item.file_entry());
        }
    });
    let dismissed_search_root = blurred_root.clone();
    let dismissed_search_button = search_button.clone();
    let dismiss_search = Rc::new(move || {
        dismissed_search_root.set_blurred(false);
        dismissed_search_button.remove_css_class("active");
    });
    let search_dialog = SearchDialog::new(activate_search_result, dismiss_search);
    let result_tabs = trails.clone();
    let activate_results = activate_tab;
    search_dialog.set_open_results(Rc::new(move |uri| {
        let location = Location::uri(uri);
        match result_tabs.create("Search", location, TrailViewState::default()) {
            Ok(id) => {
                if let Some(trail) = result_tabs.all().into_iter().find(|trail| trail.id == id) {
                    activate_results(trail);
                }
            }
            Err(error) => tracing::warn!(%error, "unable to create search results tab"),
        }
    }));
    window_overlay.add_overlay(&search_dialog.widget());
    let saved_search_layer = saved_searches::install(&window_overlay, &blurred_root, &search_dialog, &sidebar);
    let shown_search = search_dialog.clone();
    let search_browser = controller.clone();
    let search_blurred_root = blurred_root.clone();
    let saved_layer_for_search = saved_search_layer.clone();
    search_button.connect_clicked(move |button| {
        if saved_layer_for_search.is_visible() {
            return;
        }
        if shown_search.is_visible() {
            shown_search.hide();
            return;
        }
        let root = search_browser
            .active_location()
            .and_then(|location| location.native_path().map(std::path::Path::to_path_buf))
            .unwrap_or_else(home_directory);
        button.add_css_class("active");
        search_blurred_root.set_blurred(true);
        shown_search.show(root);
    });
    let search_action = gio::SimpleAction::new("search", None);
    let shortcut_search = search_dialog.clone();
    let shortcut_browser = controller.clone();
    let shortcut_search_button = search_button.clone();
    let shortcut_search_root = blurred_root.clone();
    let saved_layer_for_shortcut = saved_search_layer.clone();
    search_action.connect_activate(move |_, _| {
        if saved_layer_for_shortcut.is_visible() {
            return;
        }
        if shortcut_search.is_visible() {
            shortcut_search.hide();
        } else {
            let root = shortcut_browser
                .active_location()
                .and_then(|location| location.native_path().map(std::path::Path::to_path_buf))
                .unwrap_or_else(home_directory);
            shortcut_search_button.add_css_class("active");
            shortcut_search_root.set_blurred(true);
            shortcut_search.show(root);
        }
    });
    window.add_action(&search_action);
    application.set_accels_for_action("win.search", &["<Control>k"]);
    let music_search_action = gio::SimpleAction::new("search-music", None);
    let music_search_dialog = search_dialog.clone();
    let music_search_root = blurred_root.clone();
    let music_search_button = search_button.clone();
    music_search_action.connect_activate(move |_, _| {
        if saved_search_layer.is_visible() {
            return;
        }
        music_search_button.add_css_class("active");
        music_search_root.set_blurred(true);
        music_search_dialog.show_music();
    });
    window.add_action(&music_search_action);
    application.set_accels_for_action("win.search-music", &["<Control><Shift>k"]);

    let weak_sidebar = Rc::downgrade(&sidebar.state);
    let weak_folder_tree = folder_tree.downgrade();
    let theme_for_refresh = theme_manager.clone();
    let refresh_sidebar: Rc<dyn Fn()> = Rc::new(move || {
        if let Some(sidebar) = weak_sidebar.upgrade() {
            sidebar.rebuild();
        }
        if let Some(folder_tree) = weak_folder_tree.upgrade() {
            folder_tree.set_visible(theme_for_refresh.show_folder_tree());
        }
    });
    let settings_layer = super::settings::build_layer(
        &browser,
        &settings,
        &blurred_root,
        theme_manager,
        refresh_sidebar,
    );
    window_overlay.add_overlay(&settings_layer);
    let shown_settings = settings_layer.clone();
    let settings_button = settings.clone();
    let settings_blurred_root = blurred_root.clone();
    settings.connect_clicked(move |_| {
        show_settings(&shown_settings, &settings_button, &settings_blurred_root);
    });
    let settings_shortcut = gtk::EventControllerKey::new();
    let shown_settings = settings_layer.clone();
    let settings_button = settings.clone();
    let shortcut_blurred_root = blurred_root.clone();
    settings_shortcut.connect_key_pressed(move |_, key, _, modifiers| {
        if key != gtk::gdk::Key::comma || !modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK)
        {
            return glib::Propagation::Proceed;
        }
        show_settings(&shown_settings, &settings_button, &shortcut_blurred_root);
        glib::Propagation::Stop
    });
    window.add_controller(settings_shortcut);
    let tab_shortcut = gtk::EventControllerKey::new();
    let shortcut_tabs = tab_bar.clone();
    tab_shortcut.connect_key_pressed(move |_, key, _, modifiers| {
        if key == gtk::gdk::Key::Tab && modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK) {
            shortcut_tabs.cycle(if modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK) {
                -1
            } else {
                1
            });
            return glib::Propagation::Stop;
        }
        if !modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK) {
            return glib::Propagation::Proceed;
        }
        if matches!(key, gtk::gdk::Key::t | gtk::gdk::Key::T) {
            shortcut_tabs.new_tab();
            return glib::Propagation::Stop;
        }
        if matches!(key, gtk::gdk::Key::w | gtk::gdk::Key::W) {
            shortcut_tabs.close_active();
            return glib::Propagation::Stop;
        }
        glib::Propagation::Proceed
    });
    window.add_controller(tab_shortcut);
    window.set_child(Some(&window_overlay));
    install_modal_focus_trap(&window);
    install_keyboard_navigation(
        &window,
        &browser,
        &sidebar_toggle,
        &preview,
        trails.clone(),
        capture_view,
        &workspace,
        &music_toggle,
        &music_player,
    );
    install_mouse_navigation(&window, &controller);
    let restored_browser = browser.clone();
    let restored_sidebar = sidebar_toggle.clone();
    let restored_tabs = tab_bar.clone();
    let restored_startup = startup_restore.clone();
    let restore_startup: Rc<dyn Fn(Trail)> = Rc::new(move |trail| {
        applying_startup.set(true);
        restored_startup.replace(startup::Restore::new(&trail));
        restored_sidebar.set_active(trail.view.sidebar_open);
        restored_browser.activate_tab(&trail);
        restored_tabs.refresh();
        applying_startup.set(false);
    });
    startup::install_fallback(
        &controller,
        trails.clone(),
        startup_restore,
        home,
        fresh_view,
        restore_startup.clone(),
    );
    restore_startup(startup.trail);
    music_toggle.set_active(startup.workspace == startup::Workspace::Music);

    let browser_controller = browser.browser();
    window.connect_destroy(move |_| {
        music_player.dispose_view();
        browser_controller.clear_observer();
        sidebar.disconnect();
    });
    window.present();
    crate::metrics::mark_window_presented();
    let weak_window = window.downgrade();
    gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(1200), move || {
        if weak_window.upgrade().is_some() {
            let _warmup = crate::playback::warmup();
        }
    });
}

pub(crate) fn install_audio_playback_coordination(
    window: &gtk::ApplicationWindow,
    preview: &PreviewDrawer,
    music_controller: Rc<super::music::controller::Controller>,
) {
    let paused_music_controller = Rc::downgrade(&music_controller);
    preview.set_audio_play_handler(Rc::new(move || {
        if let Some(controller) = paused_music_controller.upgrade() {
            controller.pause();
        }
    }));
    let paused_audio_preview = preview.clone();
    let resumed_music_controller = Rc::downgrade(&music_controller);
    let observer: Rc<dyn Fn()> = Rc::new(move || {
        if resumed_music_controller.upgrade().is_some_and(|controller| {
            controller.status() == super::music::controller::PlaybackStatus::Playing
        }) {
            paused_audio_preview.pause_audio_preview();
        }
    });
    music_controller.observe(&observer);
    let retained_observer = RefCell::new(Some(observer));
    window.connect_destroy(move |_| {
        retained_observer.take();
    });
}

fn install_mouse_navigation(window: &gtk::ApplicationWindow, browser: &Rc<Browser>) {
    let buttons = gtk::GestureClick::new();
    buttons.set_button(0);
    buttons.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak_browser = Rc::downgrade(browser);
    buttons.connect_pressed(move |gesture, _, _, _| {
        let Some(browser) = weak_browser.upgrade() else {
            return;
        };
        match mouse_navigation_delta(gesture.current_button()) {
            Some(-1) => browser.back(),
            Some(1) => browser.forward(),
            _ => return,
        }
        let _claimed = gesture.set_state(gtk::EventSequenceState::Claimed);
    });
    window.add_controller(buttons);
}

fn mouse_navigation_delta(button: u32) -> Option<i8> {
    match button {
        8 => Some(-1),
        9 => Some(1),
        _ => None,
    }
}

fn trail_browser_mode(mode: BrowserMode) -> TrailBrowserMode {
    match mode {
        BrowserMode::Columns => TrailBrowserMode::Columns,
        BrowserMode::Grid => TrailBrowserMode::Grid,
        BrowserMode::Explorer => TrailBrowserMode::Explorer,
    }
}

fn browser_mode(mode: TrailBrowserMode) -> BrowserMode {
    match mode {
        TrailBrowserMode::Columns => BrowserMode::Columns,
        TrailBrowserMode::Grid => BrowserMode::Grid,
        TrailBrowserMode::Explorer => BrowserMode::Explorer,
    }
}

fn trail_browser_density(density: BrowserDensity) -> TrailBrowserDensity {
    match density {
        BrowserDensity::Compact => TrailBrowserDensity::Compact,
        BrowserDensity::Airy => TrailBrowserDensity::Airy,
    }
}

fn browser_density(density: TrailBrowserDensity) -> BrowserDensity {
    match density {
        TrailBrowserDensity::Compact => BrowserDensity::Compact,
        TrailBrowserDensity::Airy => BrowserDensity::Airy,
    }
}

fn animate_sidebar(
    paned: &gtk::Paned,
    sidebar: &gtk::Widget,
    generation: &Rc<Cell<u64>>,
    animating: &Rc<Cell<bool>>,
    expanded: bool,
) {
    let animation_id = generation.get().saturating_add(1);
    generation.set(animation_id);
    animating.set(true);
    paned.set_shrink_start_child(true);
    let target = if expanded { SIDEBAR_WIDTH } else { 0 };
    let start = paned.position();
    if expanded {
        sidebar.set_visible(true);
    }

    if !animations_enabled() || start == target {
        paned.set_position(target);
        sidebar.set_visible(expanded);
        paned.set_shrink_start_child(!expanded);
        animating.set(false);
        return;
    }

    let started = Instant::now();
    let paned = paned.clone();
    let sidebar = sidebar.clone();
    let generation = generation.clone();
    let animating = animating.clone();
    let _tick = paned.clone().add_tick_callback(move |_, _| {
        if generation.get() != animation_id {
            return glib::ControlFlow::Break;
        }

        let progress =
            (started.elapsed().as_secs_f64() / SIDEBAR_TRANSITION.as_secs_f64()).clamp(0.0, 1.0);
        let eased = emphasized_deceleration(progress);
        let position = f64::from(start) + f64::from(target - start) * eased;
        paned.set_position(position.round() as i32);

        if progress >= 1.0 {
            paned.set_position(target);
            if !expanded {
                sidebar.set_visible(false);
            }
            paned.set_shrink_start_child(!expanded);
            animating.set(false);
            glib::ControlFlow::Break
        } else {
            glib::ControlFlow::Continue
        }
    });
}

fn install_keyboard_navigation(
    window: &gtk::ApplicationWindow,
    view: &BrowserView,
    sidebar_toggle: &gtk::ToggleButton,
    preview: &PreviewDrawer,
    trails: Rc<Trails>,
    capture_view: Rc<dyn Fn() -> TrailViewState>,
    workspace: &gtk::Stack,
    music_toggle: &gtk::ToggleButton,
    music_player: &super::music::MusicPlayer,
) {
    let keys = gtk::EventControllerKey::new();
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    let view = view.clone();
    let sidebar_toggle = sidebar_toggle.clone();
    let preview = preview.clone();
    let dialog_parent = window.clone();
    let weak_browser = Rc::downgrade(&view.browser());
    let workspace = workspace.clone();
    let music_toggle = music_toggle.clone();
    let music_player = music_player.clone();
    keys.connect_key_pressed(move |_, key, _, modifiers| {
        let Some(browser) = weak_browser.upgrade() else {
            return glib::Propagation::Proceed;
        };
        if let Some(layer) = visible_modal_layer(&dialog_parent) {
            let focus_is_inside = gtk::prelude::RootExt::focus(&dialog_parent)
                .is_some_and(|focus| focus == layer || focus.is_ancestor(&layer));
            if !focus_is_inside {
                layer.grab_focus();
                return glib::Propagation::Stop;
            }
            return glib::Propagation::Proceed;
        }
        if key == gtk::gdk::Key::Escape
            && workspace.visible_child_name().as_deref() == Some("immersive")
        {
            music_toggle.set_active(true);
            workspace.set_visible_child_name("music");
            music_player.set_workspace("music");
            return glib::Propagation::Stop;
        }
        let alt = modifiers.contains(gtk::gdk::ModifierType::ALT_MASK);
        let control = modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK);
        let shift = modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK);
        if control && matches!(key, gtk::gdk::Key::k | gtk::gdk::Key::K) {
            if let Err(error) =
                gtk::prelude::WidgetExt::activate_action(&dialog_parent, if shift { "win.search-music" } else { "win.search" }, None)
            {
                tracing::warn!(%error, "unable to activate global search shortcut");
            }
            return glib::Propagation::Stop;
        }
        if !view.widget().is_mapped() {
            return glib::Propagation::Proceed;
        }
        if key == gtk::gdk::Key::F2 && view.begin_rename() {
            return glib::Propagation::Stop;
        }
        if key == gtk::gdk::Key::Escape && view.cancel_new_folder() {
            return glib::Propagation::Stop;
        }
        if key == gtk::gdk::Key::Escape && view.cancel_rename() {
            return glib::Propagation::Stop;
        }
        if view.rename_is_active() || view.new_folder_is_active() {
            return glib::Propagation::Proceed;
        }
        if key == gtk::gdk::Key::Escape && view.dismiss_focused_filter() {
            return glib::Propagation::Stop;
        }
        if control
            && !shift
            && !alt
            && matches!(key, gtk::gdk::Key::f | gtk::gdk::Key::F)
            && view.show_filter()
        {
            return glib::Propagation::Stop;
        }
        if control && key == gtk::gdk::Key::l {
            view.begin_location_edit();
            return glib::Propagation::Stop;
        }
        if control && key == gtk::gdk::Key::b {
            sidebar_toggle.set_active(!sidebar_toggle.is_active());
            return glib::Propagation::Stop;
        }
        if view.location_has_focus() {
            if key == gtk::gdk::Key::Escape {
                view.cancel_location_edit();
                return glib::Propagation::Stop;
            }
            return glib::Propagation::Proceed;
        }
        if alt
            && !control
            && !shift
            && matches!(key, gtk::gdk::Key::Return | gtk::gdk::Key::KP_Enter)
            && view.show_focused_properties()
        {
            return glib::Propagation::Stop;
        }
        if control && shift && matches!(key, gtk::gdk::Key::n | gtk::gdk::Key::N) {
            view.create_new_folder();
            return glib::Propagation::Stop;
        }
        if control && !shift && key == gtk::gdk::Key::v {
            if view.filter_has_focus() {
                return glib::Propagation::Proceed;
            }
            view.paste();
            return glib::Propagation::Stop;
        }
        if control && !shift && key == gtk::gdk::Key::c {
            if view.filter_has_focus() {
                return glib::Propagation::Proceed;
            }
            if view.copy_selection() {
                return glib::Propagation::Stop;
            }
        }
        if control && !shift && key == gtk::gdk::Key::x {
            if view.filter_has_focus() {
                return glib::Propagation::Proceed;
            }
            if view.cut_selection() {
                return glib::Propagation::Stop;
            }
        }
        if control && !shift && key == gtk::gdk::Key::a {
            if view.filter_has_focus() {
                return glib::Propagation::Proceed;
            }
            view.select_all();
            return glib::Propagation::Stop;
        }
        if control && key == gtk::gdk::Key::h {
            browser.toggle_hidden();
            return glib::Propagation::Stop;
        }
        if key == gtk::gdk::Key::Delete && !view.filter_has_focus() && view.confirm_delete(shift) {
            return glib::Propagation::Stop;
        }
        if key == gtk::gdk::Key::space && !alt && !control && !view.filter_has_focus() {
            preview.toggle(browser.focused_entry());
            return glib::Propagation::Stop;
        }
        if key == gtk::gdk::Key::Escape && preview.is_open() {
            preview.close();
            if let Err(error) = trails.update_active_view(capture_view()) {
                tracing::warn!(%error, "unable to save Trail preview state");
            }
            return glib::Propagation::Stop;
        }
        if view.filter_has_focus() && !control && !alt {
            return glib::Propagation::Proceed;
        }
        if !control && !alt && !modifiers.contains(gtk::gdk::ModifierType::SUPER_MASK)
            && matches!(key, gtk::gdk::Key::f | gtk::gdk::Key::F)
            && !gtk::prelude::RootExt::focus(&dialog_parent).is_some_and(|focus|
                focus.is::<gtk::Editable>() || focus.is::<gtk::TextView>())
            && preview.open_fullscreen()
        {
            return glib::Propagation::Stop;
        }
        if key == gtk::gdk::Key::BackSpace && !control && !alt {
            view.navigate_up();
            return glib::Propagation::Stop;
        }
        if shift && key == gtk::gdk::Key::Up {
            browser.extend_selection(-1);
            return glib::Propagation::Stop;
        }
        if shift && key == gtk::gdk::Key::Down {
            browser.extend_selection(1);
            return glib::Propagation::Stop;
        }

        match (key, alt) {
            (gtk::gdk::Key::Left, true) => browser.back(),
            (gtk::gdk::Key::Right, true) => browser.forward(),
            (gtk::gdk::Key::Up, true) => browser.parent(),
            (gtk::gdk::Key::Home, true) => {
                browser.navigate(Location::local(home_directory()));
            }
            (gtk::gdk::Key::j | gtk::gdk::Key::Down, false) => {
                if view.view_mode() == BrowserMode::Columns || key == gtk::gdk::Key::j {
                    browser.move_selection(1)
                } else {
                    return glib::Propagation::Proceed;
                }
            }
            (gtk::gdk::Key::k | gtk::gdk::Key::Up, false) => {
                if view.view_mode() == BrowserMode::Columns || key == gtk::gdk::Key::k {
                    browser.move_selection(-1)
                } else {
                    return glib::Propagation::Proceed;
                }
            }
            (gtk::gdk::Key::h | gtk::gdk::Key::Left, false) => {
                if view.view_mode() == BrowserMode::Columns || key == gtk::gdk::Key::h {
                    view.navigate_left()
                } else {
                    return glib::Propagation::Proceed;
                }
            }
            (
                gtk::gdk::Key::l
                | gtk::gdk::Key::Right
                | gtk::gdk::Key::Return
                | gtk::gdk::Key::KP_Enter,
                false,
            ) => {
                if key == gtk::gdk::Key::Return || key == gtk::gdk::Key::KP_Enter {
                    view.activate_focused()
                } else if view.view_mode() == BrowserMode::Columns || key == gtk::gdk::Key::l {
                    view.activate_focused()
                } else {
                    return glib::Propagation::Proceed;
                }
            }
            (gtk::gdk::Key::Escape, false) => browser.escape(),
            _ => return glib::Propagation::Proceed,
        }
        glib::Propagation::Stop
    });
    window.add_controller(keys);
}

fn visible_modal_layer(window: &gtk::ApplicationWindow) -> Option<gtk::Widget> {
    let overlay = window.child().and_downcast::<gtk::Overlay>()?;
    let mut child = overlay.first_child();
    while let Some(widget) = child {
        if widget.is_visible() && widget.has_css_class("app-modal-layer") {
            return Some(widget);
        }
        child = widget.next_sibling();
    }
    None
}

fn install_modal_focus_trap(window: &gtk::ApplicationWindow) {
    window.connect_focus_widget_notify(|window| {
        let Some(layer) = visible_modal_layer(window) else {
            return;
        };
        let focus_is_inside = gtk::prelude::RootExt::focus(window)
            .is_some_and(|focus| focus == layer || focus.is_ancestor(&layer));
        if !focus_is_inside {
            layer.grab_focus();
        }
    });
}

fn build_appearance_menu(
    view: &BrowserView,
    controller: &Rc<Browser>,
    preferences: Rc<super::theme::ThemeManager>,
    trails: Rc<Trails>,
    capture_view: Rc<dyn Fn() -> TrailViewState>,
) -> gtk::MenuButton {
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.add_css_class("appearance-menu");
    append_menu_heading(&content, "VIEW");
    let current_mode = view.view_mode();
    let (list, list_check, _) = appearance_option(
        crate::assets::icons::LIST,
        "List",
        current_mode == BrowserMode::Columns,
        true,
    );
    let (grid, grid_check, _) = appearance_option(
        crate::assets::icons::GRID,
        "Grid",
        current_mode == BrowserMode::Grid,
        true,
    );
    let (explorer, explorer_check, _) = appearance_option(
        crate::assets::icons::ROWS,
        "Explorer",
        current_mode == BrowserMode::Explorer,
        true,
    );
    for (button, mode) in [
        (&list, BrowserMode::Columns),
        (&grid, BrowserMode::Grid),
        (&explorer, BrowserMode::Explorer),
    ] {
        let view = view.clone();
        let list_check = list_check.clone();
        let grid_check = grid_check.clone();
        let explorer_check = explorer_check.clone();
        let preferences = preferences.clone();
        let trails = trails.clone();
        let capture_view = capture_view.clone();
        button.connect_clicked(move |_| {
            view.set_view_mode(mode);
            preferences.set_browser_mode(mode);
            list_check.set_visible(mode == BrowserMode::Columns);
            grid_check.set_visible(mode == BrowserMode::Grid);
            explorer_check.set_visible(mode == BrowserMode::Explorer);
            if let Err(error) = trails.update_active_view(capture_view()) {
                tracing::warn!(%error, "unable to save Trail view mode");
            }
        });
    }
    content.append(&list);
    content.append(&grid);
    content.append(&explorer);

    content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    append_menu_heading(&content, "DENSITY");
    let current_density = preferences.browser_density();
    let (compact, compact_check, _) = appearance_option(
        crate::assets::icons::ROWS,
        "Compact",
        current_density == BrowserDensity::Compact,
        true,
    );
    let (airy, airy_check, _) = appearance_option(
        crate::assets::icons::ROWS,
        "Airy",
        current_density == BrowserDensity::Airy,
        true,
    );
    {
        let view = view.clone();
        let compact_check = compact_check.clone();
        let airy_check = airy_check.clone();
        let preferences = preferences.clone();
        let trails = trails.clone();
        let capture_view = capture_view.clone();
        compact.connect_clicked(move |_| {
            view.set_density(BrowserDensity::Compact);
            preferences.set_browser_density(BrowserDensity::Compact);
            compact_check.set_visible(true);
            airy_check.set_visible(false);
            if let Err(error) = trails.update_active_view(capture_view()) {
                tracing::warn!(%error, "unable to save Trail density");
            }
        });
    }
    {
        let view = view.clone();
        let trails = trails.clone();
        let capture_view = capture_view.clone();
        airy.connect_clicked(move |_| {
            view.set_density(BrowserDensity::Airy);
            preferences.set_browser_density(BrowserDensity::Airy);
            compact_check.set_visible(false);
            airy_check.set_visible(true);
            if let Err(error) = trails.update_active_view(capture_view()) {
                tracing::warn!(%error, "unable to save Trail density");
            }
        });
    }
    content.append(&compact);
    content.append(&airy);

    content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    let (hidden, hidden_check, hidden_icon) =
        appearance_option(crate::assets::icons::EYE_OFF, "Hidden files", false, true);
    let hidden_state = Rc::new(Cell::new(false));
    let weak_controller = Rc::downgrade(controller);
    let hidden_trails = trails.clone();
    let hidden_view = capture_view.clone();
    hidden.connect_clicked(move |_| {
        let shown = !hidden_state.get();
        hidden_state.set(shown);
        hidden_check.set_visible(shown);
        crate::assets::set_primary_icon(
            &hidden_icon,
            if shown {
                crate::assets::icons::EYE
            } else {
                crate::assets::icons::EYE_OFF
            },
        );
        if let Some(controller) = weak_controller.upgrade() {
            controller.toggle_hidden();
            if let Err(error) = hidden_trails.update_active_view(hidden_view()) {
                tracing::warn!(%error, "unable to save Trail hidden-file state");
            }
        }
    });
    content.append(&hidden);

    let popover = gtk::Popover::builder()
        .child(&content)
        .has_arrow(false)
        .halign(gtk::Align::End)
        .position(gtk::PositionType::Bottom)
        .build();
    popover.add_css_class("appearance-popover");
    let mode_view = view.clone();
    popover.connect_show(move |_| {
        let mode = mode_view.view_mode();
        list_check.set_visible(mode == BrowserMode::Columns);
        grid_check.set_visible(mode == BrowserMode::Grid);
        explorer_check.set_visible(mode == BrowserMode::Explorer);
    });
    let button = gtk::MenuButton::builder()
        .tooltip_text("Appearance")
        .popover(&popover)
        .build();
    let icon = crate::assets::text_icon(crate::assets::icons::LIST, 20);
    button.set_child(Some(&icon));
    button.add_css_class("header-action");
    button.connect_active_notify(move |button| {
        crate::assets::set_text_icon(
            &icon,
            if button.is_active() {
                crate::assets::icons::LIST_ACTIVE
            } else {
                crate::assets::icons::LIST
            },
        );
    });
    button
}

fn appearance_option(
    icon: &str,
    label: &str,
    checked: bool,
    sensitive: bool,
) -> (gtk::Button, gtk::Image, gtk::Image) {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let check = crate::assets::primary_icon(crate::assets::icons::CHECK, 16);
    check.set_visible(checked);
    let option = crate::assets::primary_icon(icon, 17);
    let label = gtk::Label::new(Some(label));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    row.append(&option);
    row.append(&label);
    row.append(&check);
    let button = gtk::Button::builder()
        .child(&row)
        .sensitive(sensitive)
        .build();
    button.add_css_class("appearance-option");
    button.set_has_frame(false);
    (button, check, option)
}

fn show_settings(layer: &gtk::Box, button: &gtk::Button, root: &BlurBin) {
    root.set_blurred(true);
    layer.set_visible(true);
    layer.grab_focus();
    button.add_css_class("active");
}

fn append_menu_heading(container: &gtk::Box, text: &str) {
    let heading = gtk::Label::new(Some(text));
    heading.set_xalign(0.0);
    heading.add_css_class("menu-heading");
    container.append(&heading);
}

struct SidebarState {
    widget: gtk::Box,
    view: BrowserView,
    browser: Rc<Browser>,
    volume_monitor: gio::VolumeMonitor,
    place_order: RefCell<Vec<&'static str>>,
    pinned_places: RefCell<Vec<(Location, String)>>,
    place_rows: RefCell<Vec<(Location, gtk::Button)>>,
    theme_manager: Rc<ThemeManager>,
    recent_folders: Rc<RefCell<crate::ui::recent_folders::RecentFolders>>,
    saved_searches: crate::services::saved_searches::Store,
    saved_revision: Cell<u64>,
    manage_saved_searches: RefCell<Option<Rc<dyn Fn()>>>,
}

struct SidebarView {
    widget: gtk::Widget,
    state: Rc<SidebarState>,
    handlers: RefCell<Vec<glib::SignalHandlerId>>,
    saved_updates: RefCell<Option<glib::SourceId>>,
}

impl SidebarView {
    fn disconnect(&self) {
        if let Some(source) = self.saved_updates.take() {
            source.remove();
        }
        for handler in self.handlers.take() {
            self.state.volume_monitor.disconnect(handler);
        }
    }
}

impl Drop for SidebarView {
    fn drop(&mut self) {
        self.disconnect();
    }
}

impl SidebarState {
    fn rebuild(self: &Rc<Self>) {
        while let Some(child) = self.widget.first_child() {
            self.widget.remove(&child);
        }
        self.place_rows.borrow_mut().clear();

        for place in self.place_order.borrow().clone() {
            match place {
                "home" => self.append_reorderable_place(
                    "home",
                    crate::assets::icons::HOME,
                    "Home",
                    Location::local(home_directory()),
                ),
                "trash" => self.append_trash_place(),
                id => {
                    if let Some((icon, name, directory)) = standard_place(id)
                        && let Some(path) = glib::user_special_dir(directory)
                            .filter(|path| should_show_standard_place(id, path, &home_directory()))
                    {
                        self.append_reorderable_place(id, icon, name, Location::local(path));
                    }
                }
            }
        }

        if self.theme_manager.show_recent_folders() {
            let folders: Vec<_> = self.recent_folders.borrow().iter().cloned().collect();
            if !folders.is_empty() {
                self.append_separator();
                self.append_heading("RECENT FOLDERS");
                for location in folders {
                    self.append_recent_folder_place(&location);
                }
            }
        }

        if self.theme_manager.show_recent_files() {
            self.append_separator();
            self.append_heading("RECENT");
            self.append_recent_place("Today", "modified:today");
            self.append_recent_place("Yesterday", "modified:yesterday");
        }

        if self.theme_manager.show_smart_folders() {
            self.append_smart_folders();
        } else {
            self.saved_revision.set(self.saved_searches.revision());
        }
        self.append_separator();

        let pinned = self
            .pinned_places
            .borrow()
            .iter()
            .filter(|(location, _)| !is_standard_place_location(location))
            .cloned()
            .collect::<Vec<_>>();
        self.append_separator();
        self.append_heading("PINNED");
        for (location, name) in &pinned {
            self.append_pin_insertion_zone(Some(location.clone()), false);
            self.append_pinned_place(name, location.clone());
        }
        self.append_pin_insertion_zone(None, pinned.is_empty());

        let volumes = self.volume_monitor.volumes();
        let mounts: Vec<_> = self
            .volume_monitor
            .mounts()
            .into_iter()
            .filter(|mount| !mount.is_shadowed() && mount.volume().is_none())
            .map(|mount| {
                let root = mount.root();
                let location = root
                    .path()
                    .map(Location::local)
                    .unwrap_or_else(|| Location::uri(root.uri()));
                (mount.name().to_string(), location)
            })
            .collect();
        if !volumes.is_empty() || !mounts.is_empty() {
            self.append_separator();
            self.append_heading("DEVICES");
            for volume in volumes {
                self.append_volume(volume);
            }
            for (name, location) in mounts {
                self.append_place(crate::assets::icons::HARD_DRIVE, &name, location);
            }
        }
        self.sync_active_place();
    }

    fn pin_location(self: &Rc<Self>, location: Location, name: String) {
        self.pin_locations(vec![(location, name)]);
    }

    fn pin_locations(self: &Rc<Self>, folders: Vec<(Location, String)>) {
        self.pin_locations_before(folders, None);
    }

    fn pin_locations_before(
        self: &Rc<Self>,
        folders: Vec<(Location, String)>,
        before: Option<Location>,
    ) {
        let mut places = self.pinned_places.borrow_mut();
        let folders = folders
            .into_iter()
            .filter(|(location, _)| !is_standard_place_location(location))
            .collect();
        if !insert_pinned_places(&mut places, folders, before.as_ref()) {
            return;
        }
        save_pinned_places(places.clone());
        drop(places);
        self.rebuild();
    }

    fn append_pin_insertion_zone(self: &Rc<Self>, before: Option<Location>, empty: bool) {
        let zone = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        if empty {
            zone.add_css_class("sidebar-pin-drop-zone");
            zone.set_size_request(-1, 56);
            zone.append(&crate::assets::primary_icon(crate::assets::icons::PIN, 14));
            let label = gtk::Label::new(Some("Drop folders here to pin"));
            label.set_xalign(0.0);
            label.set_wrap(true);
            zone.append(&label);
        } else {
            zone.add_css_class("sidebar-pin-insertion-zone");
            zone.set_size_request(-1, 10);
        }
        zone.set_tooltip_text(Some("Drop folders here to add or arrange pins"));
        let drop = gtk::DropTarget::new(
            gtk::gdk::FileList::static_type(),
            gtk::gdk::DragAction::COPY | gtk::gdk::DragAction::MOVE,
        );
        drop.connect_enter(|target, _, _| pin_drop_action(target));
        drop.connect_motion(|target, _, _| pin_drop_action(target));
        drop.connect_accept(|_, offered| {
            tracing::info!(formats = %offered.formats().to_string(), actions = ?offered.actions(), "pin drop offered");
            (offered.formats().contains_type(gtk::gdk::FileList::static_type())
                || offered.formats().contain_mime_type("text/uri-list"))
                && !preferred_pin_drop_action(offered.actions(), offered.drag().is_some()).is_empty()
        });
        let weak_state = Rc::downgrade(self);
        let before_files = before.clone();
        drop.connect_drop(move |_, value, _, _| {
            let Some(sources) = super::browser::locations_from_file_list_value(value) else {
                tracing::warn!("pin drop had no file locations");
                return false;
            };
            tracing::info!(sources = ?sources, "pin drop received");
            let weak_state = weak_state.clone();
            let before = before_files.clone();
            glib::MainContext::default().spawn_local(async move {
                let folders = dropped_folder_bookmarks(sources).await;
                tracing::info!(folders = ?folders, "pin drop folders resolved");
                if let Some(state) = weak_state.upgrade() {
                    state.pin_locations_before(folders, before);
                }
            });
            true
        });
        zone.add_controller(drop);
        let reorder = gtk::DropTarget::new(String::static_type(), gtk::gdk::DragAction::MOVE);
        reorder.connect_accept(|_, offered| sidebar_reorder_formats(&offered.formats()));
        let weak_state = Rc::downgrade(self);
        reorder.connect_drop(move |_, value, _, _| {
            let Ok(source) = value.get::<String>() else {
                return false;
            };
            let Some(uri) = source.strip_prefix("hermes-pin:") else {
                return false;
            };
            let location = super::browser::location_for_gio_file(&gio::File::for_uri(uri));
            let Some(state) = weak_state.upgrade() else {
                return false;
            };
            let folder = state
                .pinned_places
                .borrow()
                .iter()
                .find(|(pinned, _)| pinned == &location)
                .cloned();
            let Some(folder) = folder else {
                return false;
            };
            state.pin_locations_before(vec![folder], before.clone());
            true
        });
        zone.add_controller(reorder);
        self.widget.append(&zone);
    }

    fn install_file_drop_target(&self, row: &gtk::Button, destination: Location) {
        let actions = if destination.uri_value() == Some("trash:///") {
            gtk::gdk::DragAction::MOVE
        } else {
            gtk::gdk::DragAction::COPY | gtk::gdk::DragAction::MOVE
        };
        let drop = gtk::DropTarget::new(gtk::gdk::FileList::static_type(), actions);
        let entered_destination = destination.clone();
        drop.connect_enter(move |target, _, _| sidebar_drop_action(target, &entered_destination));
        let hovered_destination = destination.clone();
        drop.connect_motion(move |target, _, _| sidebar_drop_action(target, &hovered_destination));
        let view = self.view.clone();
        drop.connect_drop(move |target, value, _, _| {
            let Some(sources) = super::browser::locations_from_file_list_value(value) else {
                return false;
            };
            view.drop_sidebar_files(
                destination.clone(),
                sources,
                sidebar_drop_action(target, &destination),
            )
        });
        row.add_controller(drop);
    }

    fn unpin_location(self: &Rc<Self>, location: &Location) {
        if remove_pinned_place(&mut self.pinned_places.borrow_mut(), location) {
            save_pinned_places(self.pinned_places.borrow().clone());
            self.rebuild();
        }
    }

    fn sync_active_place(&self) {
        let active = self.browser.active_location();
        let rows = self.place_rows.borrow();
        let selected = rows
            .iter()
            .position(|(location, row)| {
                active.as_ref() == Some(location) && row.has_css_class("active")
            })
            .or_else(|| {
                rows.iter()
                    .position(|(location, _)| active.as_ref() == Some(location))
            });
        for (index, (_, row)) in rows.iter().enumerate() {
            if selected == Some(index) {
                row.add_css_class("active");
            } else {
                row.remove_css_class("active");
            }
        }
    }

    fn append_trash_place(self: &Rc<Self>) {
        let location = Location::uri("trash:///");
        let row = sidebar_button(crate::assets::icons::TRASH, "Trash");
        row.add_css_class("reorderable");
        row.set_cursor_from_name(Some("grab"));
        row.set_tooltip_text(Some("trash:///"));
        self.install_file_drop_target(&row, location.clone());
        self.place_rows
            .borrow_mut()
            .push((location.clone(), row.clone()));
        let weak_browser = Rc::downgrade(&self.browser);
        let sidebar = self.widget.clone();
        let selected_row = row.clone();
        row.connect_clicked(move |_| {
            select_sidebar_row(&sidebar, &selected_row);
            if let Some(browser) = weak_browser.upgrade() {
                browser.navigate(location.clone());
            }
        });

        let menu = gtk::Box::new(gtk::Orientation::Vertical, 0);
        menu.add_css_class("folder-context-menu");
        let properties = sidebar_context_option(crate::assets::icons::INFO, "Properties", false);
        let empty = sidebar_context_option(crate::assets::icons::TRASH, "Empty Trash…", true);
        empty.add_css_class("danger");
        menu.append(&properties);
        menu.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        menu.append(&empty);
        let popover = gtk::Popover::builder()
            .child(&menu)
            .autohide(true)
            .has_arrow(false)
            .build();
        popover.add_css_class("folder-context-popover");
        popover.set_parent(&row);
        let properties_popover = popover.downgrade();
        let properties_view = self.view.clone();
        properties.connect_clicked(move |_| {
            if let Some(popover) = properties_popover.upgrade() {
                popover.popdown();
            }
            properties_view.show_location_properties(&Location::uri("trash:///"));
        });
        let empty_popover = popover.downgrade();
        let empty_view = self.view.clone();
        empty.connect_clicked(move |_| {
            if let Some(popover) = empty_popover.upgrade() {
                popover.popdown();
            }
            empty_view.confirm_empty_trash();
        });
        let context = gtk::GestureClick::new();
        context.set_button(3);
        let weak_popover = popover.downgrade();
        context.connect_pressed(move |gesture, _, x, y| {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            let Some(popover) = weak_popover.upgrade() else {
                return;
            };
            popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(
                x.round() as i32,
                y.round() as i32,
                1,
                1,
            )));
            popover.popup();
        });
        row.add_controller(context);
        self.install_reorder_controllers(&row, "trash");
        self.widget.append(&row);
    }

    fn install_reorder_controllers(self: &Rc<Self>, row: &gtk::Button, id: &'static str) {
        let drag = gtk::DragSource::builder()
            .actions(gtk::gdk::DragAction::MOVE)
            .build();
        drag.connect_prepare(move |_, _, _| {
            Some(gtk::gdk::ContentProvider::for_value(
                &id.to_string().to_value(),
            ))
        });
        let dragged_row = row.clone();
        drag.connect_drag_begin(move |_, _| {
            dragged_row.add_css_class("dragging");
            dragged_row.set_cursor_from_name(Some("grabbing"));
        });
        let dragged_row = row.clone();
        drag.connect_drag_end(move |_, _, _| {
            dragged_row.remove_css_class("dragging");
            dragged_row.set_cursor_from_name(Some("grab"));
        });
        row.add_controller(drag);

        let drop = gtk::DropTarget::new(String::static_type(), gtk::gdk::DragAction::MOVE);
        drop.connect_accept(|_, offered| sidebar_reorder_formats(&offered.formats()));
        let weak_state = Rc::downgrade(self);
        let target_row = row.clone();
        drop.connect_drop(move |_, value, _, y| {
            let Ok(source) = value.get::<String>() else {
                return false;
            };
            let after = y >= f64::from(target_row.height()) / 2.0;
            if let Some(state) = weak_state.upgrade() {
                if !state.place_order.borrow().contains(&source.as_str()) {
                    return false;
                }
                state.reorder_place(&source, id, after);
                return true;
            }
            false
        });
        row.add_controller(drop);
    }

    fn append_reorderable_place(
        self: &Rc<Self>,
        id: &'static str,
        icon: &str,
        name: &str,
        location: Location,
    ) {
        let row = sidebar_button(icon, name);
        row.add_css_class("reorderable");
        row.set_cursor_from_name(Some("grab"));
        row.set_tooltip_text(Some(&location.display_path()));
        self.install_file_drop_target(&row, location.clone());
        self.place_rows
            .borrow_mut()
            .push((location.clone(), row.clone()));
        let weak_browser = Rc::downgrade(&self.browser);
        let sidebar = self.widget.clone();
        let selected_row = row.clone();
        row.connect_clicked(move |_| {
            select_sidebar_row(&sidebar, &selected_row);
            if let Some(browser) = weak_browser.upgrade() {
                browser.navigate(location.clone());
            }
        });

        self.install_reorder_controllers(&row, id);
        self.widget.append(&row);
    }

    fn append_recent_place(&self, name: &str, query: &'static str) {
        let row = sidebar_button(crate::assets::icons::DOCUMENTS, name);
        row.set_tooltip_text(Some(&format!("Files {query}")));
        let sidebar = self.widget.clone();
        let selected_row = row.clone();
        let view = self.view.clone();
        row.connect_clicked(move |_| {
            select_sidebar_row(&sidebar, &selected_row);
            let uri = format!("search://{query}");
            view.navigate_location(Location::uri(&uri));
        });
        self.widget.append(&row);
    }

    fn reorder_place(self: &Rc<Self>, source: &str, target: &str, after: bool) {
        let changed = reorder_places(&mut self.place_order.borrow_mut(), source, target, after);
        if changed {
            save_place_order(self.place_order.borrow().clone());
            self.rebuild();
        }
    }

    fn append_separator(&self) {
        let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
        separator.add_css_class("sidebar-separator");
        self.widget.append(&separator);
    }

    fn append_heading(&self, text: &str) {
        let heading = gtk::Label::new(Some(text));
        heading.add_css_class("sidebar-heading");
        heading.set_xalign(0.0);
        self.widget.append(&heading);
    }

    #[expect(
        deprecated,
        reason = "GTK 4.10 dialog replacement is unavailable in the supported runtime"
    )]
    fn append_volume(&self, volume: gio::Volume) {
        let name = volume.name().to_string();
        let row = sidebar_button(crate::assets::icons::HARD_DRIVE, &name);
        row.set_tooltip_text(Some(&name));
        if let Some(mount) = volume.get_mount() {
            let root = mount.root();
            let location = root
                .path()
                .map(Location::local)
                .unwrap_or_else(|| Location::uri(root.uri()));
            self.install_file_drop_target(&row, location.clone());
            self.place_rows.borrow_mut().push((location, row.clone()));
        }
        let weak_browser = Rc::downgrade(&self.browser);
        let sidebar = self.widget.clone();
        let selected_row = row.clone();
        let click_volume = volume.clone();
        row.connect_clicked(move |button| {
            select_sidebar_row(&sidebar, &selected_row);
            let Some(browser) = weak_browser.upgrade() else {
                return;
            };
            if let Some(mount) = click_volume.get_mount() {
                navigate_to_gio_file(&browser, &mount.root());
                return;
            }

            let window = button.root().and_downcast::<gtk::Window>();
            let operation = gtk::MountOperation::new(window.as_ref());
            let volume = click_volume.clone();
            glib::MainContext::default().spawn_local(async move {
                match volume
                    .mount_future(gio::MountMountFlags::NONE, Some(&operation))
                    .await
                {
                    Ok(()) => {
                        if let Some(mount) = volume.get_mount() {
                            navigate_to_gio_file(&browser, &mount.root());
                        }
                    }
                    Err(error) => {
                        let dialog = gtk::AlertDialog::builder()
                            .modal(true)
                            .message("Unable to mount volume")
                            .detail(error.to_string())
                            .build();
                        dialog.show(window.as_ref());
                    }
                }
            });
        });
        let can_eject = volume.can_eject()
            || volume
                .get_mount()
                .as_ref()
                .is_some_and(|mount| mount.can_eject());
        let can_unmount = volume
            .get_mount()
            .as_ref()
            .is_some_and(|mount| mount.can_unmount());
        if can_eject || can_unmount {
            let menu = gtk::Box::new(gtk::Orientation::Vertical, 0);
            menu.add_css_class("folder-context-menu");
            let action_name = if can_eject { "Eject…" } else { "Unmount…" };
            let action =
                sidebar_context_option(crate::assets::icons::HARD_DRIVE, action_name, true);
            menu.append(&action);
            let popover = gtk::Popover::builder()
                .child(&menu)
                .autohide(true)
                .has_arrow(false)
                .build();
            popover.add_css_class("folder-context-popover");
            popover.set_parent(&row);
            let weak_popover = popover.downgrade();
            let volume_action = volume.clone();
            let row_action = row.clone();
            action.connect_clicked(move |_| {
                if let Some(popover) = weak_popover.upgrade() {
                    popover.popdown();
                }
                let Some(window) = row_action.root().and_downcast::<gtk::Window>() else {
                    return;
                };
                let title = if can_eject {
                    "Eject drive?"
                } else {
                    "Unmount drive?"
                };
                let dialog = gtk::Dialog::builder()
                    .transient_for(&window)
                    .modal(true)
                    .title(title)
                    .build();
                dialog.add_css_class("drive-confirm-dialog");
                let content = dialog.content_area();
                content.add_css_class("drive-confirm-content");
                let message = gtk::Label::new(Some(&format!(
                    "{} will be disconnected from Strata.",
                    volume_action.name()
                )));
                message.set_wrap(true);
                message.set_xalign(0.0);
                content.append(&message);
                dialog.add_button("Cancel", gtk::ResponseType::Cancel);
                let confirm = dialog.add_button(
                    if can_eject { "Eject" } else { "Unmount" },
                    gtk::ResponseType::Accept,
                );
                confirm.add_css_class("suggested-action");
                let volume = volume_action.clone();
                let dialog_window = window.clone();
                dialog.connect_response(move |dialog, response| {
                    if response != gtk::ResponseType::Accept {
                        dialog.close();
                        return;
                    }
                    dialog.close();
                    let volume = volume.clone();
                    let dialog_window = dialog_window.clone();
                    glib::MainContext::default().spawn_local(async move {
                        let result = if can_eject {
                            if volume.can_eject() {
                                volume
                                    .eject_with_operation_future(
                                        gio::MountUnmountFlags::NONE,
                                        None::<&gio::MountOperation>,
                                    )
                                    .await
                            } else if let Some(mount) = volume.get_mount() {
                                mount
                                    .eject_with_operation_future(
                                        gio::MountUnmountFlags::NONE,
                                        None::<&gio::MountOperation>,
                                    )
                                    .await
                            } else {
                                return;
                            }
                        } else if let Some(mount) = volume.get_mount() {
                            mount
                                .unmount_with_operation_future(
                                    gio::MountUnmountFlags::NONE,
                                    None::<&gio::MountOperation>,
                                )
                                .await
                        } else {
                            return;
                        };
                        if let Err(error) = result {
                            let error_dialog = gtk::AlertDialog::builder()
                                .modal(true)
                                .message("Unable to disconnect drive")
                                .detail(error.to_string())
                                .build();
                            error_dialog.show(Some(&dialog_window));
                        }
                    });
                });
                dialog.present();
            });
            let context = gtk::GestureClick::new();
            context.set_button(3);
            let weak_popover = popover.downgrade();
            context.connect_pressed(move |gesture, _, x, y| {
                gesture.set_state(gtk::EventSequenceState::Claimed);
                if let Some(popover) = weak_popover.upgrade() {
                    popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(
                        x.round() as i32,
                        y.round() as i32,
                        1,
                        1,
                    )));
                    popover.popup();
                }
            });
            row.add_controller(context);
        }
        self.widget.append(&row);
    }

    fn append_pinned_place(self: &Rc<Self>, name: &str, location: Location) {
        let row = self.append_place(crate::assets::icons::FOLDER, name, location.clone());
        let drag = gtk::DragSource::builder()
            .actions(gtk::gdk::DragAction::MOVE)
            .build();
        let uri = location
            .native_path()
            .map(gio::File::for_path)
            .unwrap_or_else(|| gio::File::for_uri(location.uri_value().unwrap_or_default()))
            .uri();
        drag.connect_prepare(move |_, _, _| {
            Some(gtk::gdk::ContentProvider::for_value(&format!("hermes-pin:{uri}").to_value()))
        });
        row.add_controller(drag);
        let menu = gtk::Box::new(gtk::Orientation::Vertical, 0);
        menu.add_css_class("folder-context-menu");
        let unpin = sidebar_context_option(crate::assets::icons::PIN, "Unpin", false);
        let properties = sidebar_context_option(crate::assets::icons::INFO, "Properties", false);
        menu.append(&unpin);
        menu.append(&properties);
        let popover = gtk::Popover::builder()
            .child(&menu)
            .autohide(true)
            .has_arrow(false)
            .build();
        popover.add_css_class("folder-context-popover");
        popover.set_parent(&row);

        let weak_state = Rc::downgrade(self);
        let unpinned_location = location.clone();
        let unpin_popover = popover.downgrade();
        unpin.connect_clicked(move |_| {
            if let Some(popover) = unpin_popover.upgrade() {
                popover.popdown();
            }
            if let Some(state) = weak_state.upgrade() {
                state.unpin_location(&unpinned_location);
            }
        });
        let properties_view = self.view.clone();
        let properties_location = location;
        let properties_popover = popover.downgrade();
        properties.connect_clicked(move |_| {
            if let Some(popover) = properties_popover.upgrade() {
                popover.popdown();
            }
            properties_view.show_location_properties(&properties_location);
        });
        let context = gtk::GestureClick::new();
        context.set_button(3);
        let weak_popover = popover.downgrade();
        context.connect_pressed(move |gesture, _, x, y| {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            let Some(popover) = weak_popover.upgrade() else {
                return;
            };
            popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(
                x.round() as i32,
                y.round() as i32,
                1,
                1,
            )));
            popover.popup();
        });
        row.add_controller(context);
    }

    fn append_recent_folder_place(self: &Rc<Self>, location: &Location) {
        let name = location.display_name();
        let row = self.append_place(crate::assets::icons::FOLDER, &name, location.clone());
        let menu = gtk::Box::new(gtk::Orientation::Vertical, 0);
        menu.add_css_class("folder-context-menu");

        let pin = sidebar_context_option(crate::assets::icons::PIN, "Pin to sidebar", false);
        let remove = sidebar_context_option(crate::assets::icons::X, "Remove from recent", false);
        let clear = sidebar_context_option(crate::assets::icons::TRASH, "Clear all recent", true);
        clear.add_css_class("danger");
        let properties = sidebar_context_option(crate::assets::icons::INFO, "Properties", false);

        menu.append(&pin);
        menu.append(&remove);
        menu.append(&clear);
        menu.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
        menu.append(&properties);

        let popover = gtk::Popover::builder()
            .child(&menu)
            .autohide(true)
            .has_arrow(false)
            .build();
        popover.add_css_class("folder-context-popover");
        popover.set_parent(&row);

        let weak_state = Rc::downgrade(self);
        let recent_location = location.clone();
        let pin_name = name.clone();
        let pin_popover = popover.downgrade();
        pin.connect_clicked(move |_| {
            if let Some(popover) = pin_popover.upgrade() {
                popover.popdown();
            }
            if let Some(state) = weak_state.upgrade() {
                state.pin_location(recent_location.clone(), pin_name.clone());
            }
        });

        let weak_state = Rc::downgrade(self);
        let remove_location = location.clone();
        let remove_popover = popover.downgrade();
        remove.connect_clicked(move |_| {
            if let Some(popover) = remove_popover.upgrade() {
                popover.popdown();
            }
            if let Some(state) = weak_state.upgrade() {
                state.recent_folders.borrow_mut().remove(&remove_location);
                state.rebuild();
            }
        });

        let weak_state = Rc::downgrade(self);
        let clear_popover = popover.downgrade();
        clear.connect_clicked(move |_| {
            if let Some(popover) = clear_popover.upgrade() {
                popover.popdown();
            }
            if let Some(state) = weak_state.upgrade() {
                state.recent_folders.borrow_mut().clear();
                state.rebuild();
            }
        });

        let properties_view = self.view.clone();
        let properties_location = location.clone();
        let properties_popover = popover.downgrade();
        properties.connect_clicked(move |_| {
            if let Some(popover) = properties_popover.upgrade() {
                popover.popdown();
            }
            properties_view.show_location_properties(&properties_location);
        });

        let context = gtk::GestureClick::new();
        context.set_button(gtk::gdk::ffi::GDK_BUTTON_SECONDARY as u32);
        let weak_popover = popover.downgrade();
        context.connect_pressed(move |gesture, _, x, y| {
            gesture.set_state(gtk::EventSequenceState::Claimed);
            if let Some(popover) = weak_popover.upgrade() {
                popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(
                    x.round() as i32,
                    y.round() as i32,
                    1,
                    1,
                )));
                popover.popup();
            }
        });
        row.add_controller(context);
    }

    fn append_place(&self, icon: &str, name: &str, location: Location) -> gtk::Button {
        let row = sidebar_button(icon, name);
        row.set_tooltip_text(Some(&location.display_path()));
        self.install_file_drop_target(&row, location.clone());
        self.place_rows
            .borrow_mut()
            .push((location.clone(), row.clone()));
        let weak_browser = Rc::downgrade(&self.browser);
        let sidebar = self.widget.clone();
        let selected_row = row.clone();
        row.connect_clicked(move |_| {
            select_sidebar_row(&sidebar, &selected_row);
            if let Some(browser) = weak_browser.upgrade() {
                browser.navigate(location.clone());
            }
        });
        self.widget.append(&row);
        row
    }
}

fn sidebar_reorder_formats(formats: &gtk::gdk::ContentFormats) -> bool {
    !formats.contains_type(gtk::gdk::FileList::static_type())
        && !formats.contain_mime_type("text/uri-list")
        && formats.contains_type(String::static_type())
}

fn pin_drop_action(target: &gtk::DropTarget) -> gtk::gdk::DragAction {
    target
        .current_drop()
        .map_or(gtk::gdk::DragAction::empty(), |drop| {
            preferred_pin_drop_action(drop.actions(), drop.drag().is_some())
        })
}

fn preferred_pin_drop_action(actions: gtk::gdk::DragAction, local: bool) -> gtk::gdk::DragAction {
    if actions.contains(gtk::gdk::DragAction::COPY) {
        gtk::gdk::DragAction::COPY
    } else if local && actions.contains(gtk::gdk::DragAction::MOVE) {
        gtk::gdk::DragAction::MOVE
    } else {
        gtk::gdk::DragAction::empty()
    }
}

fn sidebar_drop_action(target: &gtk::DropTarget, destination: &Location) -> gtk::gdk::DragAction {
    let Some(drop) = target.current_drop() else {
        return gtk::gdk::DragAction::empty();
    };
    preferred_sidebar_drop_action(
        drop.actions(),
        drop.drag().is_some(),
        target.current_event_state(),
        destination.uri_value() == Some("trash:///"),
    )
}

fn preferred_sidebar_drop_action(
    actions: gtk::gdk::DragAction,
    local: bool,
    modifiers: gtk::gdk::ModifierType,
    trash: bool,
) -> gtk::gdk::DragAction {
    let requested = if modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK) {
        Some(gtk::gdk::DragAction::COPY)
    } else if trash || modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK) {
        Some(gtk::gdk::DragAction::MOVE)
    } else {
        None
    };
    if let Some(action) = requested {
        if actions.contains(action) && !(trash && action == gtk::gdk::DragAction::COPY) {
            action
        } else {
            gtk::gdk::DragAction::empty()
        }
    } else {
        super::browser::preferred_file_drop_action(actions, local)
    }
}

async fn dropped_folder_bookmarks(sources: Vec<Location>) -> Vec<(Location, String)> {
    let mut folders = Vec::new();
    for location in sources {
        let Some(path) = location.native_path() else {
            continue;
        };
        if folders.iter().any(|(pinned, _)| pinned == &location) {
            continue;
        }
        let file = gio::File::for_path(path);
        if let Ok(info) = file
            .query_info_future(
                "standard::type,standard::display-name",
                gio::FileQueryInfoFlags::NONE,
                glib::Priority::DEFAULT,
            )
            .await
            && info.file_type() == gio::FileType::Directory
        {
            folders.push((location, info.display_name().to_string()));
        }
    }
    folders
}

fn select_sidebar_row(sidebar: &gtk::Box, selected: &gtk::Button) {
    let mut child = sidebar.first_child();
    while let Some(widget) = child {
        if let Ok(row) = widget.clone().downcast::<gtk::Button>() {
            row.remove_css_class("active");
        }
        child = widget.next_sibling();
    }
    selected.add_css_class("active");
}

fn reorder_places(order: &mut Vec<&'static str>, source: &str, target: &str, after: bool) -> bool {
    if source == target {
        return false;
    }
    let Some(source_index) = order.iter().position(|place| *place == source) else {
        return false;
    };
    let source = order.remove(source_index);
    let Some(target_index) = order.iter().position(|place| *place == target) else {
        order.insert(source_index, source);
        return false;
    };
    order.insert(target_index + usize::from(after), source);
    true
}

fn insert_pinned_places(
    places: &mut Vec<(Location, String)>,
    folders: Vec<(Location, String)>,
    before: Option<&Location>,
) -> bool {
    let original = places.clone();
    let mut incoming = Vec::new();
    for (location, name) in folders {
        if incoming.iter().any(|(pinned, _)| pinned == &location) {
            continue;
        }
        let name = places
            .iter()
            .find(|(pinned, _)| pinned == &location)
            .map_or(name, |(_, existing)| existing.clone());
        incoming.push((location, name));
    }
    let target = before
        .and_then(|before| places.iter().position(|(pinned, _)| pinned == before))
        .unwrap_or(places.len());
    let removed_before = places[..target]
        .iter()
        .filter(|(pinned, _)| incoming.iter().any(|(location, _)| location == pinned))
        .count();
    places.retain(|(pinned, _)| !incoming.iter().any(|(location, _)| location == pinned));
    places.splice(target - removed_before..target - removed_before, incoming);
    *places != original
}

fn remove_pinned_place(places: &mut Vec<(Location, String)>, location: &Location) -> bool {
    let original_len = places.len();
    places.retain(|(pinned, _)| pinned != location);
    places.len() != original_len
}

fn is_standard_place_location(location: &Location) -> bool {
    let Some(path) = location.native_path() else {
        return false;
    };
    if path == home_directory() {
        return true;
    }
    [
        glib::UserDirectory::Desktop,
        glib::UserDirectory::Documents,
        glib::UserDirectory::Downloads,
        glib::UserDirectory::Music,
        glib::UserDirectory::Pictures,
        glib::UserDirectory::Videos,
    ]
    .into_iter()
    .filter_map(glib::user_special_dir)
    .any(|standard| standard == path)
}

fn should_show_standard_place(id: &str, path: &std::path::Path, home: &std::path::Path) -> bool {
    id != "desktop" || path != home
}

fn standard_place(id: &str) -> Option<(&'static str, &'static str, glib::UserDirectory)> {
    match id {
        "desktop" => Some((
            crate::assets::icons::FOLDER,
            "Desktop",
            glib::UserDirectory::Desktop,
        )),
        "documents" => Some((
            crate::assets::icons::DOCUMENTS,
            "Documents",
            glib::UserDirectory::Documents,
        )),
        "downloads" => Some((
            crate::assets::icons::DOWNLOADS,
            "Downloads",
            glib::UserDirectory::Downloads,
        )),
        "pictures" => Some((
            crate::assets::icons::PICTURES,
            "Pictures",
            glib::UserDirectory::Pictures,
        )),
        "music" => Some((
            crate::assets::icons::MUSIC,
            "Music",
            glib::UserDirectory::Music,
        )),
        "videos" => Some((
            crate::assets::icons::VIDEOS,
            "Videos",
            glib::UserDirectory::Videos,
        )),
        _ => None,
    }
}

fn sidebar_context_option(icon: &str, label: &str, danger: bool) -> gtk::Button {
    let button = gtk::Button::new();
    button.add_css_class("item-context-option");
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let icon = if danger {
        crate::assets::danger_icon(icon, 15)
    } else {
        crate::assets::primary_icon(icon, 15)
    };
    icon.add_css_class("item-context-icon");
    let label = gtk::Label::new(Some(label));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    row.append(&icon);
    row.append(&label);
    button.set_child(Some(&row));
    button
}

fn sidebar_button(icon: &str, name: &str) -> gtk::Button {
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let image = crate::assets::primary_icon(icon, 17);
    let label = gtk::Label::new(Some(name));
    label.set_xalign(0.0);
    label.set_hexpand(true);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    content.append(&image);
    content.append(&label);

    let row = gtk::Button::builder()
        .child(&content)
        .halign(gtk::Align::Fill)
        .build();
    row.add_css_class("sidebar-row");
    row.set_has_frame(false);
    row
}

fn navigate_to_gio_file(browser: &Rc<Browser>, file: &gio::File) {
    let location = file
        .path()
        .map(Location::local)
        .unwrap_or_else(|| Location::uri(file.uri()));
    browser.navigate(location);
}

fn build_sidebar(view: BrowserView, theme_manager: Rc<ThemeManager>) -> SidebarView {
    build_sidebar_with_searches(view, theme_manager, crate::services::saved_searches::Store::shared())
}

fn build_sidebar_with_searches(
    view: BrowserView,
    theme_manager: Rc<ThemeManager>,
    saved_searches: crate::services::saved_searches::Store,
) -> SidebarView {
    let widget = gtk::Box::new(gtk::Orientation::Vertical, 2);
    widget.add_css_class("sidebar");
    let scroller = gtk::ScrolledWindow::builder()
        .child(&widget)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .width_request(SIDEBAR_WIDTH)
        .build();
    scroller.add_css_class("sidebar-scroll");
    let volume_monitor = gio::VolumeMonitor::get();
    let recent_folders = Rc::new(RefCell::new(
        crate::ui::recent_folders::RecentFolders::load(),
    ));
    let state = Rc::new(SidebarState {
        widget,
        browser: view.browser(),
        view,
        volume_monitor,
        place_order: RefCell::new(load_place_order()),
        pinned_places: RefCell::new(load_pinned_places()),
        place_rows: RefCell::new(Vec::new()),
        theme_manager,
        recent_folders,
        saved_searches,
        saved_revision: Cell::new(0),
        manage_saved_searches: RefCell::new(None),
    });

    let weak = Rc::downgrade(&state);
    state.browser.observe(move |event| {
        if let Some(state) = weak.upgrade() {
            match event {
                BrowserEvent::ColumnAdded { .. } | BrowserEvent::ColumnsTruncated { .. } => {
                    if state.theme_manager.show_recent_folders()
                        && let Some(active_location) = state.browser.active_location()
                    {
                        let limit = state.theme_manager.recent_folders_limit() as usize;
                        state
                            .recent_folders
                            .borrow_mut()
                            .record(&active_location, limit);
                        state.rebuild();
                    }
                    state.sync_active_place();
                }
                _ => {
                    state.sync_active_place();
                }
            }
        }
    });

    let mut handlers = Vec::new();
    let weak = Rc::downgrade(&state);
    handlers.push(state.volume_monitor.connect_mount_added(move |_, _| {
        if let Some(state) = weak.upgrade() {
            state.rebuild();
        }
    }));
    let weak = Rc::downgrade(&state);
    handlers.push(state.volume_monitor.connect_mount_removed(move |_, _| {
        if let Some(state) = weak.upgrade() {
            state.rebuild();
        }
    }));
    let weak = Rc::downgrade(&state);
    handlers.push(state.volume_monitor.connect_mount_changed(move |_, _| {
        if let Some(state) = weak.upgrade() {
            state.rebuild();
        }
    }));
    let weak = Rc::downgrade(&state);
    handlers.push(state.volume_monitor.connect_volume_added(move |_, _| {
        if let Some(state) = weak.upgrade() {
            state.rebuild();
        }
    }));
    let weak = Rc::downgrade(&state);
    handlers.push(state.volume_monitor.connect_volume_removed(move |_, _| {
        if let Some(state) = weak.upgrade() {
            state.rebuild();
        }
    }));
    let weak = Rc::downgrade(&state);
    handlers.push(state.volume_monitor.connect_volume_changed(move |_, _| {
        if let Some(state) = weak.upgrade() {
            state.rebuild();
        }
    }));
    state.rebuild();

    let weak = Rc::downgrade(&state);
    let saved_updates = glib::timeout_add_local(Duration::from_millis(150), move || {
        let Some(state) = weak.upgrade() else {
            return glib::ControlFlow::Break;
        };
        state.refresh_saved_searches();
        glib::ControlFlow::Continue
    });
    SidebarView {
        widget: scroller.upcast(),
        state,
        handlers: RefCell::new(handlers),
        saved_updates: RefCell::new(Some(saved_updates)),
    }
}

fn pinned_places_path() -> PathBuf {
    glib::user_config_dir().join("gtk-3.0/bookmarks")
}

fn place_order_path() -> PathBuf {
    glib::user_config_dir().join("strata/sidebar-order")
}

fn load_place_order() -> Vec<&'static str> {
    const DEFAULT: [&str; 8] = [
        "home",
        "trash",
        "desktop",
        "documents",
        "downloads",
        "music",
        "pictures",
        "videos",
    ];
    let mut order = std::fs::read_to_string(place_order_path())
        .ok()
        .map(|contents| {
            contents
                .lines()
                .filter_map(|line| match line {
                    "home" => Some("home"),
                    "trash" => Some("trash"),
                    "desktop" => Some("desktop"),
                    "documents" => Some("documents"),
                    "downloads" => Some("downloads"),
                    "music" => Some("music"),
                    "pictures" => Some("pictures"),
                    "videos" => Some("videos"),
                    _ => None,
                })
                .collect::<Vec<&'static str>>()
        })
        .unwrap_or_default();
    for id in DEFAULT {
        if !order.contains(&id) {
            order.push(id);
        }
    }
    order
}

fn save_place_order(order: Vec<&'static str>) {
    let path = place_order_path();
    gio::spawn_blocking(move || {
        let Some(parent) = path.parent() else {
            return;
        };
        if std::fs::create_dir_all(parent).is_ok() {
            let _ = std::fs::write(path, format!("{}\n", order.join("\n")));
        }
    });
}

fn load_pinned_places() -> Vec<(Location, String)> {
    std::fs::read_to_string(pinned_places_path())
        .map(|contents| parse_pinned_places(&contents))
        .unwrap_or_default()
}

fn parse_pinned_places(contents: &str) -> Vec<(Location, String)> {
    let mut places = Vec::new();
    for line in contents.lines() {
        let (uri, label) = line
            .split_once(' ')
            .map_or((line, None), |(uri, label)| (uri, Some(label)));
        if uri.is_empty() {
            continue;
        }
        let file = gio::File::for_uri(uri);
        let location = file
            .path()
            .map(Location::local)
            .unwrap_or_else(|| Location::uri(uri));
        if places
            .iter()
            .any(|(existing, _): &(Location, String)| existing == &location)
        {
            continue;
        }
        let name = label
            .filter(|label| !label.is_empty())
            .map(str::to_owned)
            .unwrap_or_else(|| location.display_name());
        places.push((location, name));
    }
    places
}

fn save_pinned_places(places: Vec<(Location, String)>) {
    let path = pinned_places_path();
    gio::spawn_blocking(move || {
        let Some(parent) = path.parent() else {
            return;
        };
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
        let mut contents = String::new();
        for (location, name) in places {
            let uri = location
                .native_path()
                .map(gio::File::for_path)
                .map(|file| file.uri().to_string())
                .or_else(|| location.uri_value().map(str::to_owned));
            if let Some(uri) = uri {
                let label = name.replace(['\n', '\r'], " ");
                contents.push_str(&format!("{uri} {label}\n"));
            }
        }
        let _result = std::fs::write(path, contents);
    });
}

fn home_directory() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

#[cfg(test)]
mod tests;

fn load_styles() {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(include_str!("../style.css"));

    if let Some(display) = gtk::gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}
