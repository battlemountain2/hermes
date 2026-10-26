// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    cell::{Cell, RefCell},
    fs, io,
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

use gtk::{gdk, gio, glib, prelude::*};
use serde::{Deserialize, Serialize};
use sourceview5::prelude::BufferExt as _;

mod noctalia;

thread_local! {
    static SHARED_MANAGER: RefCell<std::rc::Weak<ThemeManager>> = const { RefCell::new(std::rc::Weak::new()) };
    static SOURCE_STYLE_PATH_INSTALLED: Cell<bool> = const { Cell::new(false) };
    static SOURCE_BUFFERS: RefCell<Vec<glib::WeakRef<sourceview5::Buffer>>> = const { RefCell::new(Vec::new()) };
}

const BUILTIN_THEMES: [(&str, &str); 8] = [
    (
        "azure-glow",
        include_str!("../../data/themes/azure-glow.toml"),
    ),
    (
        "tokyo-night",
        include_str!("../../data/themes/tokyo-night.toml"),
    ),
    (
        "catppuccin",
        include_str!("../../data/themes/catppuccin.toml"),
    ),
    (
        "everforest",
        include_str!("../../data/themes/everforest.toml"),
    ),
    (
        "rose-pine",
        include_str!("../../data/themes/rose-pine.toml"),
    ),
    ("graphite", include_str!("../../data/themes/graphite.toml")),
    ("paper", include_str!("../../data/themes/paper.toml")),
    ("midnight", include_str!("../../data/themes/midnight.toml")),
];

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ThemeTokens {
    pub name: String,
    pub background: String,
    pub surface: String,
    pub text: String,
    pub accent: String,
    #[serde(default = "default_danger")]
    pub danger: String,
    pub muted: String,
    pub highlight: String,
    pub border: String,
    pub dim_text: String,
}

#[derive(Clone, Debug)]
pub struct Theme {
    pub id: String,
    pub tokens: ThemeTokens,
    pub custom: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Preferences {
    mode: String,
    theme: String,
    #[serde(default = "default_enabled")]
    single_click_previews: bool,
    #[serde(default = "default_enabled")]
    hover_previews: bool,
    #[serde(default)]
    search_open_files_directly: bool,
    #[serde(default)]
    show_recent_files: bool,
    #[serde(default = "default_enabled")]
    show_recent_folders: bool,
    #[serde(default)]
    show_folder_tree: bool,
    #[serde(default)]
    show_smart_folders: bool,
    #[serde(default = "default_recent_folders_limit")]
    recent_folders_limit: u32,
    #[serde(default = "default_browser_mode")]
    browser_mode: String,
    #[serde(default = "default_browser_density")]
    browser_density: String,
    #[serde(default = "default_interface_font")]
    interface_font: String,
    #[serde(default = "default_font_size")]
    interface_font_size: u32,
    #[serde(default)]
    context_menu: super::context_menu::Options,
    #[serde(default = "default_enabled")]
    audio_continuity: bool,
    #[serde(default = "default_enabled")]
    audio_spin_animation: bool,
    #[serde(default = "default_spectrum_bands")]
    audio_spectrum_bands: u32,
    #[serde(default = "default_spectrum_decay")]
    audio_spectrum_decay: String,
    #[serde(default)]
    online_lyrics: bool,
    #[serde(default = "default_enabled")]
    search_exclude_hidden: bool,
    #[serde(default = "default_enabled")]
    search_exclude_build_artifacts: bool,
    #[serde(default = "default_enabled")]
    search_reconcile_periodically: bool,
    #[serde(default = "default_startup_location")]
    startup_location: String,
    #[serde(default = "default_confirm_delete")]
    confirm_permanent_delete: bool,
    #[serde(default)]
    reduce_motion: bool,
    #[serde(default = "default_date_format")]
    date_format: String,
    #[serde(default = "default_size_unit")]
    size_unit: String,
    #[serde(default)]
    show_hidden_default: bool,
    #[serde(default = "default_enabled")]
    remember_folder_views: bool,
    #[serde(default = "default_max_preview_size")]
    max_preview_size_mb: u32,
    #[serde(default = "default_enabled")]
    preview_audio: bool,
    #[serde(default = "default_enabled")]
    preview_images: bool,
    #[serde(default = "default_enabled")]
    preview_videos: bool,
    #[serde(default = "default_enabled")]
    preview_pdf: bool,
    #[serde(default = "default_enabled")]
    preview_spreadsheets: bool,
    #[serde(default = "default_enabled")]
    preview_code: bool,
    #[serde(default = "default_music_player_layout")]
    music_player_layout: String,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            mode: "theme".to_owned(),
            theme: "azure-glow".to_owned(),
            single_click_previews: true,
            hover_previews: true,
            search_open_files_directly: false,
            show_recent_files: false,
            show_recent_folders: true,
            show_folder_tree: false,
            show_smart_folders: false,
            recent_folders_limit: default_recent_folders_limit(),
            browser_mode: default_browser_mode(),
            browser_density: default_browser_density(),
            interface_font: default_interface_font(),
            interface_font_size: default_font_size(),
            context_menu: super::context_menu::Options::default(),
            audio_continuity: true,
            audio_spin_animation: true,
            audio_spectrum_bands: default_spectrum_bands(),
            audio_spectrum_decay: default_spectrum_decay(),
            online_lyrics: false,
            search_exclude_hidden: true,
            search_exclude_build_artifacts: true,
            search_reconcile_periodically: true,
            startup_location: default_startup_location(),
            confirm_permanent_delete: default_confirm_delete(),
            reduce_motion: false,
            date_format: default_date_format(),
            size_unit: default_size_unit(),
            show_hidden_default: false,
            remember_folder_views: true,
            max_preview_size_mb: default_max_preview_size(),
            preview_audio: true,
            preview_images: true,
            preview_videos: true,
            preview_pdf: true,
            preview_spreadsheets: true,
            preview_code: true,
            music_player_layout: default_music_player_layout(),
        }
    }
}

fn default_music_player_layout() -> String {
    "top".to_owned()
}

fn default_startup_location() -> String {
    "home".to_owned()
}

fn default_confirm_delete() -> bool {
    true
}

fn default_date_format() -> String {
    "relative".to_owned()
}

fn default_size_unit() -> String {
    "decimal".to_owned()
}

fn default_max_preview_size() -> u32 {
    50
}

fn default_enabled() -> bool {
    true
}

fn default_spectrum_bands() -> u32 {
    32
}

fn default_spectrum_decay() -> String {
    "studio".to_owned()
}

fn default_recent_folders_limit() -> u32 {
    4
}

fn default_browser_mode() -> String {
    "columns".to_owned()
}

fn default_browser_density() -> String {
    "compact".to_owned()
}

fn default_interface_font() -> String {
    "maple".to_owned()
}

fn default_font_size() -> u32 {
    13
}

pub(super) const INTERFACE_FONTS: [(&str, &str); 4] = [
    ("maple", "Maple Mono"),
    ("jetbrains", "JetBrains Mono"),
    ("sans", "System sans-serif"),
    ("mono", "System monospace"),
];

fn normalized_font(font: &str) -> &'static str {
    INTERFACE_FONTS
        .iter()
        .find(|(id, _)| *id == font)
        .map_or("maple", |(id, _)| id)
}

fn migrate_theme_id(id: &str) -> &str {
    if id == "omarchy-light" { "paper" } else { id }
}

pub struct ThemeManager {
    provider: gtk::CssProvider,
    themes: RefCell<Vec<Theme>>,
    preferences: RefCell<Preferences>,
    applied_tokens: RefCell<Option<ThemeTokens>>,
    omarchy_available: bool,
    omarchy_monitor: RefCell<Option<gio::FileMonitor>>,
    pending_omarchy_refresh: RefCell<Option<glib::SourceId>>,
    noctalia: RefCell<noctalia::Palette>,
    noctalia_monitor: RefCell<Option<(PathBuf, gio::FileMonitor)>>,
    pending_noctalia_refresh: RefCell<Option<glib::SourceId>>,
    noctalia_poll: RefCell<Option<glib::SourceId>>,
    previewing: Cell<bool>,
    layout_observers: RefCell<Vec<Rc<dyn Fn()>>>,
    lyrics_observers: RefCell<Vec<std::rc::Weak<dyn Fn()>>>,
}

impl ThemeManager {
    pub fn shared() -> Rc<Self> {
        SHARED_MANAGER.with(|shared| {
            if let Some(manager) = shared.borrow().upgrade() {
                return manager;
            }
            let manager = Self::load();
            shared.replace(Rc::downgrade(&manager));
            manager
        })
    }

    fn load() -> Rc<Self> {
        let mut themes = builtins();
        themes.extend(load_custom_themes());
        let omarchy_available = load_omarchy_theme().is_some();
        let mut preferences = read_preferences().unwrap_or_default();
        if !themes.iter().any(|theme| theme.id == preferences.theme) {
            preferences.theme = migrate_theme_id(&preferences.theme).to_owned();
        }
        if !themes.iter().any(|theme| theme.id == preferences.theme) {
            preferences.theme = "azure-glow".to_owned();
        }
        if preferences.mode == "omarchy" && !omarchy_available {
            preferences.mode = "theme".to_owned();
        } else if !settings_path().is_file() && omarchy_available {
            preferences.mode = "omarchy".to_owned();
        }

        super::motion::set_reduce_motion(preferences.reduce_motion);
        super::formatting::configure(&preferences.date_format, &preferences.size_unit);
        super::audio_preferences::configure(
            preferences.audio_spin_animation,
            preferences.audio_spectrum_bands,
            &preferences.audio_spectrum_decay,
        );
        let manager = Rc::new(Self {
            provider: gtk::CssProvider::new(),
            themes: RefCell::new(themes),
            preferences: RefCell::new(preferences),
            applied_tokens: RefCell::new(None),
            omarchy_available,
            omarchy_monitor: RefCell::new(None),
            pending_omarchy_refresh: RefCell::new(None),
            noctalia: RefCell::new(noctalia::Palette::new(noctalia::palette_path(
                &glib::user_config_dir(),
            ))),
            noctalia_monitor: RefCell::new(None),
            pending_noctalia_refresh: RefCell::new(None),
            noctalia_poll: RefCell::new(None),
            previewing: Cell::new(false),
            layout_observers: RefCell::new(Vec::new()),
            lyrics_observers: RefCell::new(Vec::new()),
        });
        manager.noctalia.borrow_mut().refresh();
        manager.install_provider();
        manager.apply_selected();
        manager.monitor_omarchy();
        manager.monitor_noctalia();
        manager
    }

    pub fn themes(&self) -> Vec<Theme> {
        self.themes.borrow().clone()
    }

    pub fn is_omarchy_available(&self) -> bool {
        self.omarchy_available
    }

    pub fn follows_omarchy(&self) -> bool {
        self.preferences.borrow().mode == "omarchy"
    }

    pub fn follows_noctalia(&self) -> bool {
        self.preferences.borrow().mode == "noctalia"
    }

    pub fn is_noctalia_available(&self) -> bool {
        self.noctalia.borrow().last_valid.is_some()
    }

    pub fn selected_id(&self) -> String {
        self.preferences.borrow().theme.clone()
    }

    pub fn single_click_previews(&self) -> bool {
        self.preferences.borrow().single_click_previews
    }

    pub fn hover_previews(&self) -> bool {
        self.preferences.borrow().hover_previews
    }

    pub fn set_hover_previews(&self, enabled: bool) {
        self.preferences.borrow_mut().hover_previews = enabled;
        self.save_preferences();
    }

    pub fn set_single_click_previews(&self, enabled: bool) {
        self.preferences.borrow_mut().single_click_previews = enabled;
        self.save_preferences();
    }

    pub fn search_open_files_directly(&self) -> bool {
        self.preferences.borrow().search_open_files_directly
    }

    pub fn set_search_open_files_directly(&self, enabled: bool) {
        self.preferences.borrow_mut().search_open_files_directly = enabled;
        self.save_preferences();
    }

    pub fn show_recent_files(&self) -> bool {
        self.preferences.borrow().show_recent_files
    }

    pub(super) fn context_menu_options(&self) -> super::context_menu::Options {
        self.preferences.borrow().context_menu.clone()
    }

    pub(super) fn set_context_menu_options(&self, options: super::context_menu::Options) {
        self.preferences.borrow_mut().context_menu = options;
        self.save_preferences();
    }

    pub fn set_show_recent_files(&self, enabled: bool) {
        self.preferences.borrow_mut().show_recent_files = enabled;
        self.save_preferences();
    }

    pub fn show_recent_folders(&self) -> bool {
        self.preferences.borrow().show_recent_folders
    }

    pub fn set_show_recent_folders(&self, show: bool) {
        self.preferences.borrow_mut().show_recent_folders = show;
        self.save_preferences();
    }

    pub fn show_folder_tree(&self) -> bool {
        self.preferences.borrow().show_folder_tree
    }

    pub fn set_show_folder_tree(&self, show: bool) {
        self.preferences.borrow_mut().show_folder_tree = show;
        self.save_preferences();
    }

    pub fn show_smart_folders(&self) -> bool {
        self.preferences.borrow().show_smart_folders
    }

    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "Saved search visibility toggle and test helper")
    )]
    pub fn set_show_smart_folders(&self, show: bool) {
        self.preferences.borrow_mut().show_smart_folders = show;
        self.save_preferences();
    }

    pub fn recent_folders_limit(&self) -> u32 {
        self.preferences.borrow().recent_folders_limit
    }

    pub fn set_recent_folders_limit(&self, limit: u32) {
        self.preferences.borrow_mut().recent_folders_limit = limit;
        self.save_preferences();
    }

    pub fn audio_spin_animation(&self) -> bool {
        self.preferences.borrow().audio_spin_animation
    }

    pub(crate) fn online_lyrics(&self) -> bool {
        self.preferences.borrow().online_lyrics
    }

    pub(crate) fn set_online_lyrics(&self, enabled: bool) {
        if self.online_lyrics() == enabled {
            return;
        }
        self.preferences.borrow_mut().online_lyrics = enabled;
        self.save_preferences();
        let callbacks: Vec<_> = {
            let mut observers = self.lyrics_observers.borrow_mut();
            observers.retain(|observer| observer.strong_count() > 0);
            observers
                .iter()
                .filter_map(std::rc::Weak::upgrade)
                .collect()
        };
        for callback in callbacks {
            callback();
        }
    }

    pub(crate) fn observe_online_lyrics(&self, callback: &Rc<dyn Fn()>) {
        let mut observers = self.lyrics_observers.borrow_mut();
        observers.retain(|observer| observer.strong_count() > 0);
        let weak = Rc::downgrade(callback);
        if !observers.iter().any(|observer| observer.ptr_eq(&weak)) {
            observers.push(weak);
        }
    }

    pub fn set_audio_spin_animation(&self, enabled: bool) {
        if self.audio_spin_animation() == enabled {
            return;
        }
        self.preferences.borrow_mut().audio_spin_animation = enabled;
        self.refresh_audio_preferences();
        self.save_preferences();
    }

    pub fn audio_spectrum_bands(&self) -> u32 {
        if self.preferences.borrow().audio_spectrum_bands == 16 {
            16
        } else {
            32
        }
    }

    pub fn set_audio_spectrum_bands(&self, columns: u32) {
        if !matches!(columns, 16 | 32) || self.preferences.borrow().audio_spectrum_bands == columns
        {
            return;
        }
        self.preferences.borrow_mut().audio_spectrum_bands = columns;
        self.refresh_audio_preferences();
        self.save_preferences();
    }

    pub fn audio_spectrum_decay(&self) -> String {
        super::audio_preferences::Decay::from_id(&self.preferences.borrow().audio_spectrum_decay)
            .id()
            .to_owned()
    }

    pub fn set_audio_spectrum_decay(&self, decay: &str) {
        if !matches!(decay, "studio" | "snappy" | "smooth")
            || self.preferences.borrow().audio_spectrum_decay == decay
        {
            return;
        }
        self.preferences.borrow_mut().audio_spectrum_decay = decay.to_owned();
        self.refresh_audio_preferences();
        self.save_preferences();
    }

    fn refresh_audio_preferences(&self) {
        super::audio_preferences::configure(
            self.audio_spin_animation(),
            self.audio_spectrum_bands(),
            &self.audio_spectrum_decay(),
        );
    }

    pub fn search_exclude_hidden(&self) -> bool {
        self.preferences.borrow().search_exclude_hidden
    }

    pub fn search_exclude_build_artifacts(&self) -> bool {
        self.preferences.borrow().search_exclude_build_artifacts
    }

    pub fn search_reconcile_periodically(&self) -> bool {
        self.preferences.borrow().search_reconcile_periodically
    }

    pub fn startup_location(&self) -> String {
        match self.preferences.borrow().startup_location.as_str() {
            "music" => "music",
            "last" => "last",
            _ => "home",
        }
        .to_owned()
    }

    pub fn set_startup_location(&self, location: &str) {
        if !matches!(location, "home" | "music" | "last") {
            return;
        }
        self.preferences.borrow_mut().startup_location = location.to_owned();
        self.save_preferences();
    }

    pub fn confirm_permanent_delete(&self) -> bool {
        self.preferences.borrow().confirm_permanent_delete
    }

    pub fn set_confirm_permanent_delete(&self, enabled: bool) {
        self.preferences.borrow_mut().confirm_permanent_delete = enabled;
        self.save_preferences();
    }

    pub fn reduce_motion(&self) -> bool {
        self.preferences.borrow().reduce_motion
    }

    pub fn set_reduce_motion(&self, reduced: bool) {
        self.preferences.borrow_mut().reduce_motion = reduced;
        super::motion::set_reduce_motion(reduced);
        self.save_preferences();
    }

    pub fn date_format(&self) -> String {
        if self.preferences.borrow().date_format == "iso" {
            "iso"
        } else {
            "relative"
        }
        .to_owned()
    }

    pub fn set_date_format(&self, format: &str) {
        if !matches!(format, "relative" | "iso") || self.preferences.borrow().date_format == format
        {
            return;
        }
        self.preferences.borrow_mut().date_format = format.to_owned();
        super::formatting::configure(&self.date_format(), &self.size_unit());
        self.save_preferences();
    }

    pub fn size_unit(&self) -> String {
        if self.preferences.borrow().size_unit == "binary" {
            "binary"
        } else {
            "decimal"
        }
        .to_owned()
    }

    pub fn set_size_unit(&self, unit: &str) {
        if !matches!(unit, "decimal" | "binary") || self.preferences.borrow().size_unit == unit {
            return;
        }
        self.preferences.borrow_mut().size_unit = unit.to_owned();
        super::formatting::configure(&self.date_format(), &self.size_unit());
        self.save_preferences();
    }

    pub fn show_hidden_default(&self) -> bool {
        self.preferences.borrow().show_hidden_default
    }

    pub fn set_show_hidden_default(&self, enabled: bool) {
        self.preferences.borrow_mut().show_hidden_default = enabled;
        self.save_preferences();
    }

    pub fn remember_folder_views(&self) -> bool {
        self.preferences.borrow().remember_folder_views
    }

    pub fn set_remember_folder_views(&self, remember: bool) {
        self.preferences.borrow_mut().remember_folder_views = remember;
        self.save_preferences();
    }

    pub fn max_preview_size_mb(&self) -> u32 {
        self.preferences.borrow().max_preview_size_mb
    }

    pub fn set_max_preview_size_mb(&self, size: u32) {
        self.preferences.borrow_mut().max_preview_size_mb = size;
        self.save_preferences();
    }

    pub fn preview_audio(&self) -> bool {
        self.preferences.borrow().preview_audio
    }

    pub fn set_preview_audio(&self, enabled: bool) {
        self.preferences.borrow_mut().preview_audio = enabled;
        self.save_preferences();
    }

    pub fn preview_images(&self) -> bool {
        self.preferences.borrow().preview_images
    }

    pub fn set_preview_images(&self, enabled: bool) {
        self.preferences.borrow_mut().preview_images = enabled;
        self.save_preferences();
    }

    pub fn preview_videos(&self) -> bool {
        self.preferences.borrow().preview_videos
    }

    pub fn set_preview_videos(&self, enabled: bool) {
        self.preferences.borrow_mut().preview_videos = enabled;
        self.save_preferences();
    }

    pub fn preview_pdf(&self) -> bool {
        self.preferences.borrow().preview_pdf
    }

    pub fn set_preview_pdf(&self, enabled: bool) {
        self.preferences.borrow_mut().preview_pdf = enabled;
        self.save_preferences();
    }

    pub fn preview_spreadsheets(&self) -> bool {
        self.preferences.borrow().preview_spreadsheets
    }

    pub fn set_preview_spreadsheets(&self, enabled: bool) {
        self.preferences.borrow_mut().preview_spreadsheets = enabled;
        self.save_preferences();
    }

    pub fn preview_code(&self) -> bool {
        self.preferences.borrow().preview_code
    }

    pub fn set_preview_code(&self, enabled: bool) {
        self.preferences.borrow_mut().preview_code = enabled;
        self.save_preferences();
    }

    pub fn observe_layout<F: Fn() + 'static>(&self, callback: F) {
        self.layout_observers.borrow_mut().push(Rc::new(callback));
    }

    pub fn music_player_layout(&self) -> String {
        self.preferences.borrow().music_player_layout.clone()
    }

    pub fn set_music_player_layout(&self, layout: &str) {
        self.preferences.borrow_mut().music_player_layout = layout.to_owned();
        self.save_preferences();
        let observers = self.layout_observers.borrow().clone();
        for observer in observers {
            observer();
        }
    }

    pub fn interface_font(&self) -> &str {
        normalized_font(&self.preferences.borrow().interface_font)
    }

    pub fn set_interface_font(&self, font: &str) {
        let font = normalized_font(font);
        self.preferences.borrow_mut().interface_font = font.to_owned();
        self.apply_selected();
        self.save_preferences();
    }

    pub fn interface_font_size(&self) -> u32 {
        self.preferences.borrow().interface_font_size.clamp(10, 22)
    }

    pub fn set_interface_font_size(&self, size: u32) {
        self.preferences.borrow_mut().interface_font_size = size.clamp(10, 22);
        self.apply_selected();
        self.save_preferences();
    }

    pub fn browser_mode(&self) -> super::browser_modes::BrowserMode {
        match self.preferences.borrow().browser_mode.as_str() {
            "grid" => super::browser_modes::BrowserMode::Grid,
            "explorer" => super::browser_modes::BrowserMode::Explorer,
            _ => super::browser_modes::BrowserMode::Columns,
        }
    }

    pub fn set_browser_mode(&self, mode: super::browser_modes::BrowserMode) {
        self.preferences.borrow_mut().browser_mode = match mode {
            super::browser_modes::BrowserMode::Columns => "columns",
            super::browser_modes::BrowserMode::Grid => "grid",
            super::browser_modes::BrowserMode::Explorer => "explorer",
        }
        .to_owned();
        self.save_preferences();
    }

    pub fn browser_density(&self) -> super::browser_modes::BrowserDensity {
        match self.preferences.borrow().browser_density.as_str() {
            "airy" => super::browser_modes::BrowserDensity::Airy,
            _ => super::browser_modes::BrowserDensity::Compact,
        }
    }

    pub fn set_browser_density(&self, density: super::browser_modes::BrowserDensity) {
        self.preferences.borrow_mut().browser_density = match density {
            super::browser_modes::BrowserDensity::Compact => "compact",
            super::browser_modes::BrowserDensity::Airy => "airy",
        }
        .to_owned();
        self.save_preferences();
    }

    pub fn select_theme(&self, id: &str) {
        if !self.themes.borrow().iter().any(|theme| theme.id == id) {
            return;
        }
        {
            let mut preferences = self.preferences.borrow_mut();
            preferences.mode = "theme".to_owned();
            preferences.theme = id.to_owned();
        }
        self.previewing.set(false);
        self.apply_selected();
        self.save_preferences();
    }

    pub fn set_follow_omarchy(&self, enabled: bool) {
        if enabled && !self.omarchy_available {
            return;
        }
        self.set_follow_mode("omarchy", enabled);
    }

    pub fn set_follow_noctalia(&self, enabled: bool) {
        self.set_follow_mode("noctalia", enabled);
    }

    fn set_follow_mode(&self, mode: &str, enabled: bool) {
        {
            let mut preferences = self.preferences.borrow_mut();
            if enabled {
                preferences.mode = mode.to_owned();
            } else if preferences.mode == mode {
                preferences.mode = "theme".to_owned();
            } else {
                return;
            }
        }
        self.previewing.set(false);
        self.apply_selected();
        self.save_preferences();
    }

    pub fn preview(&self, tokens: &ThemeTokens) {
        if validate_tokens(tokens).is_ok() {
            self.previewing.set(true);
            self.apply_tokens(tokens);
        }
    }

    pub fn cancel_preview(&self) {
        if self.previewing.replace(false) {
            self.apply_selected();
        }
    }

    pub fn save_custom_theme(&self, tokens: ThemeTokens) -> io::Result<String> {
        validate_tokens(&tokens)
            .map_err(|message| io::Error::new(io::ErrorKind::InvalidInput, message))?;
        let base = slugify(&tokens.name);
        if base.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Enter a theme name",
            ));
        }
        let directory = themes_directory();
        fs::create_dir_all(&directory)?;
        let mut id = base.clone();
        let mut suffix = 2;
        while self.themes.borrow().iter().any(|theme| theme.id == id) {
            id = format!("{base}-{suffix}");
            suffix += 1;
        }
        fs::write(
            directory.join(format!("{id}.toml")),
            toml::to_string_pretty(&tokens).map_err(io::Error::other)?,
        )?;

        let mut themes = self.themes.borrow_mut();
        if let Some(theme) = themes
            .iter_mut()
            .find(|theme| theme.id == id && theme.custom)
        {
            theme.tokens = tokens;
        } else {
            themes.push(Theme {
                id: id.clone(),
                tokens,
                custom: true,
            });
        }
        drop(themes);
        self.select_theme(&id);
        Ok(id)
    }

    pub fn starter_tokens(&self) -> ThemeTokens {
        self.current_tokens().unwrap_or_else(azure_tokens)
    }

    pub fn waveform_colors(&self) -> Option<(gdk::RGBA, gdk::RGBA)> {
        let tokens = self.applied_tokens.borrow();
        let tokens = tokens.as_ref()?;
        Some((
            gdk::RGBA::parse(&tokens.accent).ok()?,
            gdk::RGBA::parse(&tokens.muted).ok()?,
        ))
    }

    pub fn applied_tokens(&self) -> Option<ThemeTokens> {
        self.applied_tokens.borrow().clone()
    }

    fn install_provider(&self) {
        if let Some(display) = gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display,
                &self.provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
            );
        }
    }

    fn apply_selected(&self) {
        if self.follows_noctalia() {
            self.noctalia.borrow_mut().refresh();
        }
        if self.follows_omarchy() {
            if let Some(tokens) = load_omarchy_theme() {
                self.apply_tokens(&tokens);
            }
            return;
        }
        if let Some(tokens) = self.current_tokens() {
            self.apply_tokens(&tokens);
        }
    }

    pub(crate) fn current_tokens(&self) -> Option<ThemeTokens> {
        if self.follows_noctalia()
            && let Some(tokens) = self.noctalia.borrow().last_valid.clone()
        {
            return Some(tokens);
        }
        let id = self.preferences.borrow().theme.clone();
        self.themes
            .borrow()
            .iter()
            .find(|theme| theme.id == id)
            .map(|theme| theme.tokens.clone())
    }

    fn apply_tokens(&self, tokens: &ThemeTokens) {
        self.provider.load_from_string(&format!(
            "{}\nwindow, popover, .search-dialog, .search-filter-popover, .search-filter-panel, .app-modal-layer {{ font-size: {}px; }}\n",
            tokens_css(tokens, self.interface_font()),
            self.interface_font_size()
        ));
        self.applied_tokens.replace(Some(tokens.clone()));
        crate::assets::set_primary_icon_color(&tokens.accent);
        crate::assets::set_text_icon_color(&tokens.text);
        crate::assets::set_on_accent_icon_color(on_accent_color(tokens));
        crate::assets::set_danger_icon_color(&tokens.danger);
        install_source_style_scheme(tokens);
    }

    fn save_preferences(&self) {
        let path = settings_path();
        let result = (|| -> io::Result<()> {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let value =
                toml::to_string_pretty(&*self.preferences.borrow()).map_err(io::Error::other)?;
            fs::write(path, value)
        })();
        if let Err(error) = result {
            tracing::warn!(%error, "unable to save theme preference");
        }
    }

    fn monitor_omarchy(self: &Rc<Self>) {
        if !self.omarchy_available {
            return;
        }
        let file = gio::File::for_path(omarchy_state_dir());
        let Ok(monitor) =
            file.monitor_directory(gio::FileMonitorFlags::NONE, gio::Cancellable::NONE)
        else {
            return;
        };
        let weak = Rc::downgrade(self);
        monitor.connect_changed(move |_, file, other_file, _| {
            if !is_omarchy_theme_event(file)
                && !other_file
                    .as_ref()
                    .is_some_and(|file| is_omarchy_theme_event(file))
            {
                return;
            }
            let Some(manager) = weak.upgrade() else {
                return;
            };
            if let Some(pending) = manager.pending_omarchy_refresh.borrow_mut().take() {
                pending.remove();
            }
            let weak = weak.clone();
            let refresh = glib::timeout_add_local_once(Duration::from_millis(75), move || {
                let Some(manager) = weak.upgrade() else {
                    return;
                };
                manager.pending_omarchy_refresh.borrow_mut().take();
                if manager.follows_omarchy() && !manager.previewing.get() {
                    manager.apply_selected();
                }
            });
            manager.pending_omarchy_refresh.replace(Some(refresh));
        });
        self.omarchy_monitor.replace(Some(monitor));
    }

    fn monitor_noctalia(self: &Rc<Self>) {
        self.ensure_noctalia_monitor();
        let weak = Rc::downgrade(self);
        let poll = glib::timeout_add_local(Duration::from_secs(2), move || {
            let Some(manager) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            manager.ensure_noctalia_monitor();
            if manager.pending_noctalia_refresh.borrow().is_none() {
                manager.refresh_noctalia();
            }
            glib::ControlFlow::Continue
        });
        self.noctalia_poll.replace(Some(poll));
    }

    fn ensure_noctalia_monitor(self: &Rc<Self>) {
        let Some(directory) = noctalia::watch_directory(&self.noctalia.borrow().path) else {
            return;
        };
        if self
            .noctalia_monitor
            .borrow()
            .as_ref()
            .is_some_and(|(path, monitor)| *path == directory && !monitor.is_cancelled())
        {
            return;
        }
        if let Some((_, monitor)) = self.noctalia_monitor.borrow_mut().take() {
            monitor.cancel();
        }
        let Ok(monitor) = gio::File::for_path(&directory)
            .monitor_directory(gio::FileMonitorFlags::WATCH_MOVES, gio::Cancellable::NONE)
        else {
            return;
        };
        let weak = Rc::downgrade(self);
        monitor.connect_changed(move |_, file, other, _| {
            let Some(manager) = weak.upgrade() else {
                return;
            };
            let path = manager.noctalia.borrow().path.clone();
            if !file
                .path()
                .is_some_and(|changed| noctalia::relevant_event(&path, &changed))
                && !other
                    .and_then(gio::File::path)
                    .is_some_and(|changed| noctalia::relevant_event(&path, &changed))
            {
                return;
            }
            if file
                .path()
                .is_some_and(|changed| changed != path && noctalia::relevant_event(&path, &changed))
                && let Some((_, monitor)) = manager.noctalia_monitor.borrow_mut().take()
            {
                monitor.cancel();
            }
            if let Some(pending) = manager.pending_noctalia_refresh.borrow_mut().take() {
                pending.remove();
            }
            let weak = weak.clone();
            let refresh = glib::timeout_add_local_once(Duration::from_millis(150), move || {
                let Some(manager) = weak.upgrade() else {
                    return;
                };
                manager.pending_noctalia_refresh.borrow_mut().take();
                manager.ensure_noctalia_monitor();
                manager.refresh_noctalia();
            });
            manager.pending_noctalia_refresh.replace(Some(refresh));
        });
        self.noctalia_monitor.replace(Some((directory, monitor)));
    }

    fn refresh_noctalia(&self) {
        let changed = self.noctalia.borrow_mut().refresh();
        if changed && self.follows_noctalia() && !self.previewing.get() {
            self.apply_selected();
        }
    }
}

impl Drop for ThemeManager {
    fn drop(&mut self) {
        for source in [
            &self.pending_omarchy_refresh,
            &self.pending_noctalia_refresh,
            &self.noctalia_poll,
        ] {
            if let Some(source) = source.borrow_mut().take() {
                source.remove();
            }
        }
        if let Some((_, monitor)) = self.noctalia_monitor.borrow_mut().take() {
            monitor.cancel();
        }
        if let Some(monitor) = self.omarchy_monitor.borrow_mut().take() {
            monitor.cancel();
        }
    }
}

fn is_omarchy_theme_event(file: &gio::File) -> bool {
    file.path()
        .as_deref()
        .and_then(Path::file_name)
        .is_some_and(|name| name == "theme" || name == "theme.name")
}

fn builtins() -> Vec<Theme> {
    BUILTIN_THEMES
        .iter()
        .filter_map(|(id, source)| {
            toml::from_str(source).ok().map(|tokens| Theme {
                id: (*id).to_owned(),
                tokens,
                custom: false,
            })
        })
        .collect()
}

fn azure_tokens() -> ThemeTokens {
    toml::from_str(BUILTIN_THEMES[0].1).unwrap_or_else(|_| ThemeTokens {
        name: "Azure Glow".to_owned(),
        background: "#0c1a2b".to_owned(),
        surface: "#122438".to_owned(),
        text: "#c9deed".to_owned(),
        accent: "#4fd6ff".to_owned(),
        danger: default_danger(),
        muted: "#1e3a52".to_owned(),
        highlight: "#244d68".to_owned(),
        border: "#315b75".to_owned(),
        dim_text: "#6f8da3".to_owned(),
    })
}

fn load_custom_themes() -> Vec<Theme> {
    let Ok(entries) = fs::read_dir(themes_directory()) else {
        return Vec::new();
    };
    let mut themes: Vec<_> = entries
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .path()
                .extension()
                .is_some_and(|extension| extension == "toml")
        })
        .filter_map(|entry| {
            let id = entry.path().file_stem()?.to_string_lossy().into_owned();
            let source = fs::read_to_string(entry.path()).ok()?;
            let tokens: ThemeTokens = toml::from_str(&source).ok()?;
            validate_tokens(&tokens).ok()?;
            Some(Theme {
                id,
                tokens,
                custom: true,
            })
        })
        .collect();
    themes.sort_by(|left, right| left.tokens.name.cmp(&right.tokens.name));
    themes
}

fn read_preferences() -> Option<Preferences> {
    toml::from_str(&fs::read_to_string(settings_path()).ok()?).ok()
}

fn load_omarchy_theme() -> Option<ThemeTokens> {
    let state = omarchy_state_dir();
    let name = fs::read_to_string(state.join("theme.name")).ok()?;
    let colors = fs::read_to_string(state.join("theme/colors.toml")).ok()?;
    tokens_from_quattro(name.trim(), &colors)
}

fn tokens_from_quattro(name: &str, source: &str) -> Option<ThemeTokens> {
    let values: toml::Value = toml::from_str(source).ok()?;
    let get = |key: &str| {
        values
            .get(key)
            .and_then(toml::Value::as_str)
            .map(str::to_owned)
    };
    let source_background = get("background")?;
    let text = get("foreground")?;
    let accent = get("accent")?;
    let selection = get("selection").unwrap_or_else(|| accent.clone());
    let shadow = get("color8").unwrap_or_else(|| source_background.clone());
    Some(ThemeTokens {
        name: title_case_slug(name),
        background: blend(&source_background, &shadow, 0.35),
        surface: blend(&source_background, &shadow, 0.65),
        muted: blend(&shadow, &text, 0.10),
        highlight: blend(&shadow, &selection, 0.10),
        border: blend(&shadow, &text, 0.36),
        dim_text: blend(&source_background, &text, 0.62),
        text,
        accent,
        danger: get("color1").unwrap_or_else(default_danger),
    })
}

fn default_danger() -> String {
    "#e5484d".to_owned()
}

fn validate_tokens(tokens: &ThemeTokens) -> Result<(), &'static str> {
    if tokens.name.trim().is_empty() {
        return Err("Enter a theme name");
    }
    for color in [
        &tokens.background,
        &tokens.surface,
        &tokens.text,
        &tokens.accent,
        &tokens.danger,
        &tokens.muted,
        &tokens.highlight,
        &tokens.border,
        &tokens.dim_text,
    ] {
        if gdk::RGBA::parse(color).is_err() {
            return Err("Every color must be a valid CSS color");
        }
    }
    Ok(())
}

fn source_style_scheme() -> Option<sourceview5::StyleScheme> {
    sourceview5::StyleSchemeManager::default().scheme("strata-current")
}

pub(super) fn register_source_buffer(buffer: &sourceview5::Buffer) {
    buffer.set_style_scheme(source_style_scheme().as_ref());
    SOURCE_BUFFERS.with(|buffers| {
        let mut buffers = buffers.borrow_mut();
        buffers.retain(|buffer| buffer.upgrade().is_some());
        let weak = glib::WeakRef::new();
        weak.set(Some(buffer));
        buffers.push(weak);
    });
}

fn install_source_style_scheme(tokens: &ThemeTokens) {
    let directory = glib::user_cache_dir().join("strata").join("source-styles");
    if let Err(error) = fs::create_dir_all(&directory).and_then(|()| {
        fs::write(
            directory.join("strata-current.xml"),
            source_style_scheme_xml(tokens),
        )
    }) {
        tracing::warn!(%error, "unable to write preview syntax style");
        return;
    }

    let manager = sourceview5::StyleSchemeManager::default();
    SOURCE_STYLE_PATH_INSTALLED.with(|installed| {
        if !installed.replace(true) {
            manager.append_search_path(&directory.to_string_lossy());
        }
    });
    manager.force_rescan();
    let scheme = manager.scheme("strata-current");
    SOURCE_BUFFERS.with(|buffers| {
        buffers.borrow_mut().retain(|buffer| {
            let Some(buffer) = buffer.upgrade() else {
                return false;
            };
            buffer.set_style_scheme(scheme.as_ref());
            true
        });
    });
}

fn source_style_scheme_xml(tokens: &ThemeTokens) -> String {
    let string = blend(&tokens.accent, &tokens.text, 0.48);
    let constant = blend(&tokens.accent, &tokens.text, 0.18);
    let type_color = blend(&tokens.accent, &tokens.text, 0.24);
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<style-scheme id="strata-current" _name="Hermes Current Theme" version="1.0">
  <color name="background" value="{}"/>
  <color name="surface" value="{}"/>
  <color name="text" value="{}"/>
  <color name="accent" value="{}"/>
  <color name="selection" value="{}"/>
  <color name="dim" value="{}"/>
  <color name="string" value="{}"/>
  <color name="constant" value="{}"/>
  <color name="type" value="{}"/>
  <style name="text" foreground="text" background="surface"/>
  <style name="selection" foreground="background" background="accent"/>
  <style name="cursor" foreground="accent"/>
  <style name="current-line" background="background"/>
  <style name="line-numbers" foreground="dim" background="background"/>
  <style name="def:comment" foreground="dim" italic="true"/>
  <style name="def:shebang" foreground="dim" bold="true"/>
  <style name="def:string" foreground="string"/>
  <style name="def:constant" foreground="constant"/>
  <style name="def:special-char" foreground="constant"/>
  <style name="def:identifier" foreground="text"/>
  <style name="def:statement" foreground="accent" bold="true"/>
  <style name="def:type" foreground="type" bold="true"/>
  <style name="def:preprocessor" foreground="type"/>
  <style name="def:heading" foreground="accent" bold="true"/>
  <style name="def:link-destination" foreground="string" underline="single"/>
  <style name="def:error" foreground="background" background="accent" bold="true"/>
</style-scheme>
"#,
        tokens.background,
        tokens.surface,
        tokens.text,
        tokens.accent,
        tokens.highlight,
        tokens.dim_text,
        string,
        constant,
        type_color,
    )
}

fn tokens_css(tokens: &ThemeTokens, interface_font: &str) -> String {
    let font_family = match normalized_font(interface_font) {
        "jetbrains" => r#""JetBrains Mono", "JetBrainsMono Nerd Font", monospace"#,
        "sans" => "sans-serif",
        "mono" => "monospace",
        _ => r#""Maple Mono Normal", "JetBrains Mono", monospace"#,
    };
    format!(
        "@define-color theme_bg {};\n@define-color theme_surface {};\n@define-color theme_text {};\n@define-color theme_accent {};\n@define-color theme_danger {};\n@define-color theme_muted {};\n@define-color theme_highlight {};\n@define-color theme_border {};\n@define-color theme_dim_text {};\nwindow, popover, .search-dialog, .search-filter-popover, .search-filter-panel, .app-modal-layer, .preview-text, .preview-text text, .monospace, .search-filter-shortcut, .grid-thumbnail-value {{ font-family: {}; }}\n@define-color theme_on_accent {};\n",
        tokens.background,
        tokens.surface,
        tokens.text,
        tokens.accent,
        tokens.danger,
        tokens.muted,
        tokens.highlight,
        tokens.border,
        tokens.dim_text,
        font_family,
        on_accent_color(tokens),
    )
}

fn on_accent_color(tokens: &ThemeTokens) -> &str {
    let luminance = |color: &str| {
        let color = gdk::RGBA::parse(color).ok()?;
        let linear = |channel: f32| {
            let channel = f64::from(channel);
            if channel <= 0.04045 {
                channel / 12.92
            } else {
                ((channel + 0.055) / 1.055).powf(2.4)
            }
        };
        Some(
            0.2126 * linear(color.red())
                + 0.7152 * linear(color.green())
                + 0.0722 * linear(color.blue()),
        )
    };
    let Some(accent) = luminance(&tokens.accent) else {
        return &tokens.text;
    };
    let contrast = |color: &str| {
        luminance(color).map_or(0.0, |value| {
            (accent.max(value) + 0.05) / (accent.min(value) + 0.05)
        })
    };
    if contrast(&tokens.background) >= contrast(&tokens.text) {
        &tokens.background
    } else {
        &tokens.text
    }
}

fn blend(left: &str, right: &str, amount: f64) -> String {
    let parse = |value: &str| u32::from_str_radix(value.trim_start_matches('#'), 16).ok();
    let (Some(left), Some(right)) = (parse(left), parse(right)) else {
        return right.to_owned();
    };
    let channel = |shift| {
        let a = f64::from((left >> shift) & 0xff_u32);
        let b = f64::from((right >> shift) & 0xff_u32);
        (a + (b - a) * amount).round() as u32
    };
    format!("#{:02x}{:02x}{:02x}", channel(16), channel(8), channel(0))
}

fn slugify(name: &str) -> String {
    name.trim()
        .to_lowercase()
        .chars()
        .fold(String::new(), |mut slug, character| {
            if character.is_ascii_alphanumeric() {
                slug.push(character);
            } else if !slug.is_empty() && !slug.ends_with('-') {
                slug.push('-');
            }
            slug
        })
        .trim_end_matches('-')
        .to_owned()
}

fn title_case_slug(slug: &str) -> String {
    slug.split('-')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn config_directory() -> PathBuf {
    gtk::glib::user_config_dir().join("strata")
}
fn settings_path() -> PathBuf {
    config_directory().join("settings.toml")
}
fn themes_directory() -> PathBuf {
    config_directory().join("themes")
}
fn omarchy_state_dir() -> PathBuf {
    gtk::glib::home_dir().join(".local/state/omarchy/current")
}

#[cfg(test)]
mod tests;
