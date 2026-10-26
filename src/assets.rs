// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    cell::RefCell,
    collections::HashMap,
    ffi::CString,
    fs, io,
    io::Cursor,
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
};

use gtk::{gdk, gio, glib, glib::object::ObjectType};

pub mod icons {
    pub const PLAY: &str = "strata-play";
    pub const PAUSE: &str = "strata-pause";
    pub const VOLUME: &str = "strata-volume-2";
    pub const MUTED: &str = "strata-volume-x";
    pub const MAXIMIZE: &str = "strata-maximize";
    pub const MINIMIZE: &str = "strata-minimize";
    pub const ARROW_DOWN: &str = "strata-arrow-down";
    pub const ARROW_DOWN_WIDE_NARROW: &str = "strata-arrow-down-wide-narrow";
    pub const ARROW_LEFT: &str = "strata-arrow-left";
    pub const ARROW_RIGHT: &str = "strata-arrow-right";
    pub const ARROW_UP: &str = "strata-arrow-up";
    pub const ARROW_UP_NARROW_WIDE: &str = "strata-arrow-up-narrow-wide";
    pub const CHECK: &str = "strata-check";
    pub const CHECK_ON_PRIMARY: &str = "strata-check-on-primary";
    pub const CHEVRON_RIGHT: &str = "strata-chevron-right";
    pub const COPY: &str = "strata-copy";
    pub const DOCUMENTS: &str = "strata-file-text";
    pub const DOWNLOADS: &str = "strata-download";
    pub const EYE: &str = "strata-eye";
    pub const EYE_OFF: &str = "strata-eye-off";
    pub const EXTERNAL_LINK: &str = "strata-external-link";
    pub const FILE_ARCHIVE: &str = "strata-file-archive";
    pub const FILE_CODE: &str = "strata-file-code";
    pub const FOLDER: &str = "strata-folder";
    pub const FOLDER_FILLED: &str = "strata-folder-filled";
    pub const HARD_DRIVE: &str = "strata-hard-drive";
    pub const HEART: &str = "strata-heart";
    pub const HEART_SELECTED: &str = "strata-heart-selected";
    pub const INFO: &str = "strata-info";
    pub const FUNNEL: &str = "strata-funnel";
    pub const GRID: &str = "strata-grid";
    pub const HOME: &str = "strata-house";
    pub const LIST: &str = "strata-list";
    pub const LIST_ACTIVE: &str = "strata-list-active";
    pub const KEYBOARD: &str = "strata-keyboard";
    pub const MONITOR: &str = "strata-monitor";
    pub const MUSIC: &str = "strata-music";
    pub const PALETTE: &str = "strata-palette";
    pub const PANEL_LEFT: &str = "strata-panel-left-symbolic";
    pub const PENCIL: &str = "strata-pencil";
    pub const PIN: &str = "strata-pin";
    pub const PLUS: &str = "strata-plus";
    pub const PICTURES: &str = "strata-image";
    pub const ROWS: &str = "strata-rows";
    pub const REPEAT: &str = "strata-repeat";
    pub const REPEAT_ONE: &str = "strata-repeat-1";
    pub const SCISSORS: &str = "strata-scissors";
    pub const SEARCH: &str = "strata-search";
    pub const SETTINGS: &str = "strata-settings";
    pub const SETTINGS_2: &str = "strata-settings-2";
    pub const SHUFFLE: &str = "strata-shuffle";
    pub const SLIDERS: &str = "strata-sliders-horizontal";
    pub const TERMINAL: &str = "strata-terminal";
    pub const TRASH: &str = "strata-trash";
    pub const VIDEOS: &str = "strata-video";
    pub const X: &str = "strata-x";
    pub const STAR: &str = "strata-star";
    pub const STAR_FILLED: &str = "strata-star-filled";
}

const FONT_VERSION: &str = "jetbrains-2.304-maple-7.9";
const ICON_TEXTURE_CACHE_LIMIT: usize = 256;
const JETBRAINS_MONO: &[u8] = include_bytes!("../data/fonts/JetBrainsMono[wght].ttf");
const MAPLE_MONO: &[u8] = include_bytes!("../data/fonts/MapleMonoNormal[wght].ttf");

struct PrimaryIcon {
    image: glib::WeakRef<gtk::Image>,
    name: String,
}

#[derive(Default)]
struct IconRegistry {
    entries: HashMap<usize, PrimaryIcon>,
    registrations_since_cleanup: usize,
}

thread_local! {
    static PRIMARY_ICON_COLOR: RefCell<String> = RefCell::new("#8bc9eb".to_owned());
    static PRIMARY_ICONS: RefCell<IconRegistry> = RefCell::new(IconRegistry::default());
    static TEXT_ICON_COLOR: RefCell<String> = RefCell::new("#c9deed".to_owned());
    static TEXT_ICONS: RefCell<IconRegistry> = RefCell::new(IconRegistry::default());
    static ON_ACCENT_ICON_COLOR: RefCell<Option<String>> = const { RefCell::new(None) };
    static ON_ACCENT_ICONS: RefCell<IconRegistry> = RefCell::new(IconRegistry::default());
    static DANGER_ICON_COLOR: RefCell<String> = RefCell::new("#e5484d".to_owned());
    static DANGER_ICONS: RefCell<IconRegistry> = RefCell::new(IconRegistry::default());
    static ICON_TEXTURES: RefCell<HashMap<(String, String), gdk::Texture>> = RefCell::new(HashMap::new());
}

pub fn prepare() -> Result<(), Box<dyn std::error::Error>> {
    gio::resources_register_include!("strata.gresource")?;

    let font_directory = glib::user_cache_dir()
        .join("strata")
        .join("fonts")
        .join(FONT_VERSION);
    fs::create_dir_all(&font_directory)?;

    let jetbrains = font_directory.join("JetBrainsMono.ttf");
    let maple = font_directory.join("MapleMonoNormal.ttf");
    write_if_changed(&jetbrains, JETBRAINS_MONO)?;
    write_if_changed(&maple, MAPLE_MONO)?;
    register_application_fonts([jetbrains, maple])?;

    Ok(())
}

pub fn register_icon_theme() {
    if let Some(display) = gdk::Display::default() {
        gtk::IconTheme::for_display(&display).add_resource_path("/io/github/lgse/Strata/icons");
        gtk::IconTheme::for_display(&display).add_resource_path("/io/github/battlemountain2/Hermes/icons");
    }
}

pub fn primary_icon(name: &str, pixel_size: i32) -> gtk::Image {
    let image = gtk::Image::new();
    image.set_pixel_size(pixel_size);
    set_primary_icon(&image, name);
    image
}

pub fn set_primary_icon(image: &gtk::Image, name: &str) {
    ON_ACCENT_ICONS.with(|icons| icons.borrow_mut().entries.remove(&(image.as_ptr() as usize)));
    let color = PRIMARY_ICON_COLOR.with(|color| color.borrow().clone());
    apply_primary_icon(image, name, &color);
    PRIMARY_ICONS.with(|icons| register_icon(icons, image, name));
}

pub fn set_primary_icon_color(color: &str) {
    PRIMARY_ICON_COLOR.with(|current| current.replace(color.to_owned()));
    PRIMARY_ICONS.with(|icons| recolor_registered_icons(icons, color));
}

pub fn remove_primary_icon(image: &gtk::Image) {
    PRIMARY_ICONS.with(|icons| {
        icons
            .borrow_mut()
            .entries
            .remove(&(image.as_ptr() as usize));
    });
}

pub fn has_primary_icon(image: &gtk::Image) -> bool {
    PRIMARY_ICONS.with(|icons| {
        icons
            .borrow()
            .entries
            .get(&(image.as_ptr() as usize))
            .is_some_and(|icon| icon.image.upgrade().as_ref() == Some(image))
    })
}

pub fn text_icon(name: &str, pixel_size: i32) -> gtk::Image {
    let image = gtk::Image::new();
    image.set_pixel_size(pixel_size);
    set_text_icon(&image, name);
    image
}

pub fn set_text_icon(image: &gtk::Image, name: &str) {
    ON_ACCENT_ICONS.with(|icons| icons.borrow_mut().entries.remove(&(image.as_ptr() as usize)));
    let color = TEXT_ICON_COLOR.with(|color| color.borrow().clone());
    apply_primary_icon(image, name, &color);
    TEXT_ICONS.with(|icons| register_icon(icons, image, name));
}

pub fn set_text_icon_color(color: &str) {
    TEXT_ICON_COLOR.with(|current| current.replace(color.to_owned()));
    TEXT_ICONS.with(|icons| recolor_registered_icons(icons, color));
}

pub fn on_accent_icon(name: &str, pixel_size: i32) -> gtk::Image {
    let image = gtk::Image::new();
    image.set_pixel_size(pixel_size);
    set_on_accent_icon(&image, name);
    image
}

pub fn set_on_accent_icon(image: &gtk::Image, name: &str) {
    PRIMARY_ICONS.with(|icons| icons.borrow_mut().entries.remove(&(image.as_ptr() as usize)));
    TEXT_ICONS.with(|icons| icons.borrow_mut().entries.remove(&(image.as_ptr() as usize)));
    let color = ON_ACCENT_ICON_COLOR
        .with(|color| color.borrow().clone())
        .unwrap_or_else(|| TEXT_ICON_COLOR.with(|color| color.borrow().clone()));
    apply_primary_icon(image, name, &color);
    ON_ACCENT_ICONS.with(|icons| register_icon(icons, image, name));
}

pub fn set_on_accent_icon_color(color: &str) {
    ON_ACCENT_ICON_COLOR.with(|current| current.replace(Some(color.to_owned())));
    ON_ACCENT_ICONS.with(|icons| recolor_registered_icons(icons, color));
}

pub fn danger_icon(name: &str, pixel_size: i32) -> gtk::Image {
    let image = gtk::Image::new();
    image.set_pixel_size(pixel_size);
    let color = DANGER_ICON_COLOR.with(|color| color.borrow().clone());
    apply_primary_icon(&image, name, &color);
    DANGER_ICONS.with(|icons| register_icon(icons, &image, name));
    image
}

pub fn set_danger_icon_color(color: &str) {
    DANGER_ICON_COLOR.with(|current| current.replace(color.to_owned()));
    DANGER_ICONS.with(|icons| recolor_registered_icons(icons, color));
}

fn register_icon(icons: &RefCell<IconRegistry>, image: &gtk::Image, name: &str) {
    let mut icons = icons.borrow_mut();
    icons.registrations_since_cleanup += 1;
    if icons.registrations_since_cleanup >= 256 {
        icons
            .entries
            .retain(|_, icon| icon.image.upgrade().is_some());
        icons.registrations_since_cleanup = 0;
    }
    let image_ref = glib::WeakRef::new();
    image_ref.set(Some(image));
    icons.entries.insert(
        image.as_ptr() as usize,
        PrimaryIcon {
            image: image_ref,
            name: name.to_owned(),
        },
    );
}

fn recolor_registered_icons(icons: &RefCell<IconRegistry>, color: &str) {
    icons.borrow_mut().entries.retain(|_, icon| {
        let Some(image) = icon.image.upgrade() else {
            return false;
        };
        apply_primary_icon(&image, &icon.name, color);
        true
    });
}

fn apply_primary_icon(image: &gtk::Image, name: &str, color: &str) {
    if let Some(texture) = primary_icon_texture(name, color) {
        image.set_paintable(Some(&texture));
    } else {
        image.set_icon_name(Some(name));
    }
}

fn primary_icon_texture(name: &str, color: &str) -> Option<gdk::Texture> {
    let key = (name.to_owned(), color.to_owned());
    if let Some(texture) = ICON_TEXTURES.with(|textures| textures.borrow().get(&key).cloned()) {
        return Some(texture);
    }

    let path = format!("/io/github/lgse/Strata/icons/scalable/actions/{name}.svg");
    let data = gio::resources_lookup_data(&path, gio::ResourceLookupFlags::NONE).ok()?;
    let source = std::str::from_utf8(data.as_ref()).ok()?;
    let mut source = recolor_icon_source(source, color);
    if name == icons::FOLDER {
        source = source.replacen(
            "fill=\"none\"",
            &format!("fill=\"{color}\" fill-opacity=\"0.15\""),
            1,
        );
    }
    let pixbuf = gdk_pixbuf::Pixbuf::from_read(Cursor::new(source.into_bytes())).ok()?;
    let texture = gdk::Texture::for_pixbuf(&pixbuf);
    ICON_TEXTURES.with(|textures| {
        let mut textures = textures.borrow_mut();
        if textures.len() >= ICON_TEXTURE_CACHE_LIMIT {
            textures.clear();
        }
        textures.insert(key, texture.clone());
    });
    Some(texture)
}

fn recolor_icon_source(source: &str, color: &str) -> String {
    source
        .replace("#8bc9eb", color)
        .replace("#22d3ee", color)
        .replace("#2e3436", color)
}

fn write_if_changed(path: &Path, contents: &[u8]) -> io::Result<()> {
    let is_current = path
        .metadata()
        .map(|metadata| metadata.len() == contents.len() as u64)
        .unwrap_or(false);

    if !is_current {
        fs::write(path, contents)?;
    }

    Ok(())
}

#[expect(
    unsafe_code,
    reason = "Fontconfig exposes application-font registration only through its C FFI"
)]
fn register_application_fonts(
    paths: impl IntoIterator<Item = PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    // SAFETY: This no-argument Fontconfig call returns a borrowed process-global
    // configuration. We check it for null before passing it to any other FFI call.
    let config = unsafe { fontconfig_sys::FcConfigGetCurrent() };
    if config.is_null() {
        return Err("Fontconfig did not provide a current configuration".into());
    }

    for path in paths {
        let path = CString::new(path.as_os_str().as_bytes())?;

        // SAFETY: `config` was checked above. `path` is a valid, NUL-terminated C string
        // that remains alive for the call, and Fontconfig copies rather than retains it.
        let registered =
            unsafe { fontconfig_sys::FcConfigAppFontAddFile(config, path.as_ptr().cast()) };
        if registered == 0 {
            return Err("Fontconfig could not register a bundled font".into());
        }
    }

    // SAFETY: `config` is the same checked process-global configuration. Registration
    // runs during single-threaded startup before GTK/Pango creates the application's map.
    let rebuilt = unsafe { fontconfig_sys::FcConfigBuildFonts(config) };
    if rebuilt == 0 {
        return Err("Fontconfig could not rebuild the application font set".into());
    }

    Ok(())
}

#[cfg(test)]
mod tests;
