use super::*;
use crate::{
    assets::{self, icons},
    services::{FormatFamily, capabilities_by_name, classify_by_name},
};

pub(super) fn identity(item: &SearchItem) -> (&'static str, &'static str) {
    if item.is_directory {
        let music = SearchScope::MusicLibrary.root(Path::new("/"));
        return (
            icons::FOLDER,
            if item.path.starts_with(music) {
                "Music folder"
            } else {
                "Folder"
            },
        );
    }
    let extension = item
        .path
        .extension()
        .unwrap_or_default()
        .to_string_lossy()
        .to_ascii_lowercase();
    if matches!(extension.as_str(), "exe" | "appimage") {
        return (icons::MONITOR, "Application");
    }
    match classify_by_name(item.path.as_os_str()) {
        FormatFamily::Audio => (icons::MUSIC, "Song"),
        FormatFamily::Video => (icons::VIDEOS, "Video"),
        FormatFamily::Image
        | FormatFamily::Svg
        | FormatFamily::Heif
        | FormatFamily::RawImage
        | FormatFamily::GeoTiff => (icons::PICTURES, "Image"),
        FormatFamily::DesktopEntry => (icons::EXTERNAL_LINK, "Launcher"),
        FormatFamily::Archive => (icons::FILE_ARCHIVE, "Archive"),
        FormatFamily::Comic => (icons::FILE_ARCHIVE, "Book / comic"),
        FormatFamily::Pdf => (icons::DOCUMENTS, "PDF"),
        FormatFamily::OfficeDocument => (icons::DOCUMENTS, "Document"),
        FormatFamily::Spreadsheet => (icons::GRID, "Spreadsheet"),
        FormatFamily::Model => (icons::GRID, "3D model"),
        FormatFamily::PlainText if matches!(extension.as_str(), "sh" | "bash" | "zsh" | "fish") => {
            (icons::TERMINAL, "Script")
        }
        FormatFamily::PlainText => (icons::FILE_CODE, "Text / code"),
        FormatFamily::Unknown => (icons::DOCUMENTS, "File"),
    }
}

pub(super) fn music_detail(item: &SearchItem) -> String {
    let Some(tags) = item.music_tags.as_ref() else {
        return String::new();
    };
    [
        tags.artist.as_ref().or(tags.album_artist.as_ref()),
        tags.album.as_ref(),
    ]
    .into_iter()
    .flatten()
    .filter(|value| !value.is_empty())
    .cloned()
    .collect::<Vec<_>>()
    .join(" · ")
}

pub(super) fn row(item: &SearchItem, root: &Path) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    row.add_css_class("search-result");
    row.set_tooltip_text(Some(&item.path.to_string_lossy()));
    let content = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let (icon, kind) = identity(item);
    let image = assets::primary_icon(icon, 24);
    image.set_size_request(44, 44);
    image.add_css_class("search-result-art");
    content.append(&image);
    let labels = gtk::Box::new(gtk::Orientation::Vertical, 3);
    labels.set_hexpand(true);
    let title = item
        .music_tags
        .as_ref()
        .and_then(|tags| tags.title.as_deref())
        .unwrap_or(&item.name);
    let name = label(title, "search-result-name", gtk::pango::EllipsizeMode::End);
    labels.append(&name);
    let metadata = music_detail(item);
    if !metadata.is_empty() {
        labels.append(&label(
            &metadata,
            "search-result-metadata",
            gtk::pango::EllipsizeMode::End,
        ));
    }
    let relative = item.path.strip_prefix(root).unwrap_or(&item.path);
    labels.append(&label(
        &relative.to_string_lossy(),
        "search-result-path",
        gtk::pango::EllipsizeMode::Middle,
    ));
    content.append(&labels);
    let extension = if item.is_directory {
        String::new()
    } else {
        item.path
            .extension()
            .unwrap_or_default()
            .to_string_lossy()
            .chars()
            .take(12)
            .collect::<String>()
            .to_uppercase()
    };
    let badge_text = if extension.is_empty() || extension == kind {
        kind.to_owned()
    } else {
        format!("{kind} · {extension}")
    };
    let badge = label(
        &badge_text,
        "search-result-type",
        gtk::pango::EllipsizeMode::End,
    );
    badge.set_max_width_chars(20);
    badge.set_halign(gtk::Align::End);
    badge.set_valign(gtk::Align::Center);
    content.append(&badge);
    row.set_child(Some(&content));
    if item.is_directory
        || capabilities_by_name(item.path.as_os_str())
            .thumbnail
            .is_some()
        || classify_by_name(item.path.as_os_str()) == FormatFamily::DesktopEntry
    {
        lazy_thumbnail(&row, &image, item.file_entry(), icon);
    }
    row
}

fn label(value: &str, class: &str, ellipsize: gtk::pango::EllipsizeMode) -> gtk::Label {
    let label = gtk::Label::new(Some(value));
    label.add_css_class(class);
    label.set_xalign(0.0);
    label.set_ellipsize(ellipsize);
    label
}

fn lazy_thumbnail(
    row: &gtk::ListBoxRow,
    image: &gtk::Image,
    entry: crate::model::FileEntry,
    icon: &'static str,
) {
    let generation = Rc::new(Cell::new(0_u64));
    let mapped_generation = generation.clone();
    let weak_image = image.downgrade();
    row.connect_map(move |row| {
        let generation = mapped_generation.clone();
        let current = generation.get().wrapping_add(1);
        generation.set(current);
        let weak_image = weak_image.clone();
        let entry = entry.clone();
        let frames = Cell::new(0);
        row.add_tick_callback(move |row, _| {
            if generation.get() != current {
                return glib::ControlFlow::Break;
            }
            frames.set(frames.get() + 1);
            if frames.get() < 3 || row.width() <= 0 || row.height() <= 0 {
                return glib::ControlFlow::Continue;
            }
            let Some(image) = weak_image.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let Some(scroll) = row
                .ancestor(gtk::ScrolledWindow::static_type())
                .and_downcast::<gtk::ScrolledWindow>()
            else {
                return glib::ControlFlow::Continue;
            };
            if scroll.vadjustment().page_size() <= 0.0 {
                return glib::ControlFlow::Continue;
            }
            let Some(bounds) = row.compute_bounds(&scroll) else {
                return glib::ControlFlow::Continue;
            };
            if bounds.y() + bounds.height() <= 0.0 || bounds.y() >= scroll.height() as f32 {
                return glib::ControlFlow::Continue;
            }
            super::super::thumbnail::set_thumbnail_or_icon(&image, &entry, icon, 44, 44);
            glib::ControlFlow::Break
        });
    });
    image.connect_unmap(move |image| {
        generation.set(generation.get().wrapping_add(1));
        super::super::thumbnail::cancel_thumbnail(image);
    });
}
