// SPDX-License-Identifier: GPL-3.0-or-later

use crate::sandbox_helper::metadata::Details;
use crate::{
    model::{FileEntry, MetadataValue},
    sandbox::{Cancellation, ParseOperation},
    services::{FormatFamily, ThumbnailHandler, capabilities_by_name, classify_by_name},
};
use gtk::{gio, glib, prelude::*};
use std::os::unix::fs::MetadataExt;
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

pub(super) fn install(
    anchor: &impl IsA<gtk::Widget>,
    item: &gtk::ListItem,
    resolve: Rc<dyn Fn() -> Option<FileEntry>>,
) {
    let anchor = anchor.as_ref();
    anchor.set_has_tooltip(true);
    let slot = Rc::new(RefCell::new(None::<Rc<Card>>));
    let suppressed = Rc::new(Cell::new(false));
    let query_slot = slot.clone();
    let query_suppressed = suppressed.clone();
    anchor.connect_query_tooltip(move |anchor, _, _, _, tooltip| {
        if query_suppressed.get() || editing(anchor) {
            return false;
        }
        let Some(entry) = resolve() else {
            return false;
        };
        if !super::theme::ThemeManager::shared().hover_previews() {
            tooltip.set_text(Some(&entry.display_name));
            return true;
        }
        let card = query_slot
            .borrow_mut()
            .get_or_insert_with(Card::new)
            .clone();
        card.bind(entry);
        tooltip.set_custom(Some(&card.widget));
        true
    });
    let weak_slot = Rc::downgrade(&slot);
    let weak_anchor = anchor.downgrade();
    item.connect_item_notify(move |_| {
        if let Some(slot) = weak_slot.upgrade() {
            let card = slot.borrow_mut().take();
            if let Some(card) = card {
                card.cancel();
                card.widget.set_visible(false);
            }
        }
        if let Some(anchor) = weak_anchor.upgrade() {
            anchor.trigger_tooltip_query();
        }
    });
    let motion = gtk::EventControllerMotion::new();
    let entered_suppressed = suppressed.clone();
    motion.connect_enter(move |_, _, _| entered_suppressed.set(false));
    let left_slot = Rc::downgrade(&slot);
    motion.connect_leave(move |_| {
        if let Some(slot) = left_slot.upgrade()
            && let Some(card) = slot.borrow().as_ref()
        {
            card.cancel();
        }
    });
    anchor.add_controller(motion);
    let click = gtk::GestureClick::new();
    click.set_button(0);
    click.set_propagation_phase(gtk::PropagationPhase::Capture);
    let weak_anchor = anchor.downgrade();
    click.connect_pressed(move |_, _, _, _| {
        suppressed.set(true);
        if let Some(card) = slot.borrow().as_ref() {
            card.cancel();
        }
        if let Some(anchor) = weak_anchor.upgrade() {
            anchor.trigger_tooltip_query();
        }
    });
    anchor.add_controller(click);
}

#[derive(Clone)]
struct FolderTile {
    container: gtk::Box,
    image: gtk::Image,
    label: gtk::Label,
}

struct Card {
    widget: gtk::Box,
    // File view
    file_box: gtk::Box,
    image: gtk::Image,
    filename: gtk::Label,
    badge: gtk::Label,
    rating: gtk::Label,
    title: gtk::Label,
    artist: gtk::Label,
    album: gtk::Label,
    info: gtk::Label,
    details: gtk::Label,
    // Folder view
    folder_box: gtk::Box,
    folder_name: gtk::Label,
    folder_summary: gtk::Label,
    folder_cells: Vec<FolderTile>,
    // Common state
    entry: RefCell<Option<FileEntry>>,
    generation: Cell<u64>,
    cancellation: RefCell<Cancellation>,
    task: RefCell<Option<glib::JoinHandle<()>>>,
}

impl Card {
    fn new() -> Rc<Self> {
        let widget = gtk::Box::new(gtk::Orientation::Vertical, 0);
        widget.add_css_class("hermes-hover-card");
        widget.set_size_request(310, 220);
        widget.set_overflow(gtk::Overflow::Hidden);

        // --- File View ---
        let file_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let header_row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        header_row.set_hexpand(true);
        let filename = label("hover-filename", 22, 1);
        filename.set_hexpand(true);
        let rating = label("hover-rating", 6, 1);
        rating.set_halign(gtk::Align::End);
        let badge = label("hover-badge", 10, 1);
        badge.set_halign(gtk::Align::End);
        header_row.append(&filename);
        header_row.append(&rating);
        header_row.append(&badge);
        file_box.append(&header_row);

        let body = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let visual = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        visual.set_size_request(104, 104);
        let image = gtk::Image::new();
        image.set_size_request(104, 104);
        image.set_halign(gtk::Align::Center);
        image.set_valign(gtk::Align::Center);
        visual.append(&image);
        body.append(&visual);

        let text = gtk::Box::new(gtk::Orientation::Vertical, 5);
        let title = label("hover-title", 17, 1);
        let artist = label("hover-secondary", 17, 1);
        let album = label("hover-secondary", 17, 1);
        text.append(&title);
        text.append(&artist);
        text.append(&album);
        body.append(&text);
        file_box.append(&body);

        let info = label("hover-secondary", 34, 1);
        let details = label("hover-secondary", 34, 1);
        file_box.append(&info);
        file_box.append(&details);
        widget.append(&file_box);

        // --- Folder View ---
        let folder_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
        let folder_header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let folder_icon = crate::assets::primary_icon(crate::assets::icons::FOLDER, 22);
        folder_header.append(&folder_icon);
        let folder_titles = gtk::Box::new(gtk::Orientation::Vertical, 2);
        let folder_name = label("hover-filename", 26, 1);
        let folder_summary = label("hover-secondary", 26, 1);
        folder_titles.append(&folder_name);
        folder_titles.append(&folder_summary);
        folder_header.append(&folder_titles);
        folder_box.append(&folder_header);

        let folder_grid = gtk::Grid::builder()
            .column_spacing(8)
            .row_spacing(8)
            .hexpand(true)
            .vexpand(true)
            .build();
        folder_grid.add_css_class("folder-peek-grid");

        let mut folder_cells = Vec::new();
        for i in 0..4 {
            let row = i / 2;
            let col = i % 2;
            let tile_box = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            tile_box.add_css_class("folder-peek-tile");
            tile_box.set_size_request(138, 48);
            tile_box.set_hexpand(true);

            let tile_img = gtk::Image::new();
            tile_img.set_pixel_size(28);
            let tile_lbl = label("folder-tile-label", 12, 1);
            tile_lbl.set_valign(gtk::Align::Center);
            tile_lbl.set_hexpand(true);

            tile_box.append(&tile_img);
            tile_box.append(&tile_lbl);
            folder_grid.attach(&tile_box, col, row, 1, 1);
            folder_cells.push(FolderTile {
                container: tile_box,
                image: tile_img,
                label: tile_lbl,
            });
        }
        folder_box.append(&folder_grid);
        widget.append(&folder_box);

        let card = Rc::new(Self {
            widget,
            file_box,
            image,
            filename,
            badge,
            rating,
            title,
            artist,
            album,
            info,
            details,
            folder_box,
            folder_name,
            folder_summary,
            folder_cells,
            entry: RefCell::new(None),
            generation: Cell::new(0),
            cancellation: RefCell::new(Cancellation::default()),
            task: RefCell::new(None),
        });

        let weak = Rc::downgrade(&card);
        card.widget.connect_map(move |_| {
            if let Some(card) = weak.upgrade() {
                card.start();
            }
        });
        let weak = Rc::downgrade(&card);
        card.widget.connect_unmap(move |_| {
            if let Some(card) = weak.upgrade() {
                card.cancel();
            }
        });
        card
    }

    fn bind(self: &Rc<Self>, entry: FileEntry) {
        if self.entry.borrow().as_ref() == Some(&entry) {
            return;
        }
        self.cancel();

        if entry.is_directory() {
            self.file_box.set_visible(false);
            self.folder_box.set_visible(true);
            self.folder_name.set_text(&entry.display_name);
            self.folder_summary.set_text("Reading contents…");
            for tile in &self.folder_cells {
                tile.container.set_visible(false);
            }
        } else {
            self.folder_box.set_visible(false);
            self.file_box.set_visible(true);
            self.filename.set_text(&entry.display_name);
            self.title.set_text(
                entry
                    .music_tags
                    .as_ref()
                    .and_then(|tags| tags.title.as_deref())
                    .unwrap_or(&entry.display_name),
            );
            self.artist.set_text(
                entry
                    .music_tags
                    .as_ref()
                    .and_then(|tags| tags.artist.as_deref().or(tags.album_artist.as_deref()))
                    .unwrap_or(""),
            );
            self.album.set_text(
                entry
                    .music_tags
                    .as_ref()
                    .and_then(|tags| tags.album.as_deref())
                    .unwrap_or(""),
            );
            let size = match entry.size {
                MetadataValue::Known(size) => super::formatting::format_size(size),
                _ => "Size unavailable".into(),
            };
            self.info.set_text(&format!("{} · {size}", kind(&entry)));
            self.details.set_text(&match entry.modified_unix_seconds {
                MetadataValue::Known(modified) => {
                    format!("Modified {}", super::formatting::format_timestamp(modified))
                }
                _ => String::new(),
            });

            if let Some(path) = entry.location.native_path() {
                if let Some(r) = crate::services::ratings::Store::shared().get(path) {
                    let stars = "★".repeat(r as usize);
                    self.rating.set_text(&stars);
                    self.rating.set_visible(true);
                } else {
                    self.rating.set_visible(false);
                }
            } else {
                self.rating.set_visible(false);
            }

            self.badge.set_text(&kind(&entry).to_uppercase());
            super::thumbnail::show_fallback_icon(
                &self.image,
                super::browser::entry_icon(&entry),
                26,
            );
        }

        self.entry.replace(Some(entry));
        if self.widget.is_mapped() {
            self.start();
        }
    }

    fn start(self: &Rc<Self>) {
        self.cancel();
        let Some(entry) = self.entry.borrow().clone() else {
            return;
        };
        let Some(path) = entry.location.native_path().map(ToOwned::to_owned) else {
            return;
        };
        if !super::theme::ThemeManager::shared().hover_previews() {
            return;
        }

        let generation = self.generation.get();
        let cancellation = Cancellation::default();
        self.cancellation.replace(cancellation.clone());
        let weak = Rc::downgrade(self);

        if entry.is_directory() {
            let scan_cancel = cancellation.clone();
            let task = glib::MainContext::default().spawn_local(async move {
                let scan_res = gio::spawn_blocking(move || {
                    if scan_cancel.is_cancelled() {
                        return None;
                    }
                    let Ok(read_dir) = std::fs::read_dir(&path) else {
                        return None;
                    };
                    let mut total = 0;
                    let mut samples = Vec::new();
                    for child in read_dir.flatten() {
                        if scan_cancel.is_cancelled() {
                            return None;
                        }
                        let name = child.file_name().to_string_lossy().to_string();
                        if name.starts_with('.') {
                            continue;
                        }
                        total += 1;
                        if samples.len() < 4 {
                            let is_dir = child.file_type().map(|t| t.is_dir()).unwrap_or(false);
                            samples.push((name, is_dir, child.path()));
                        }
                    }
                    Some((total, samples))
                })
                .await;

                let Some(card) = weak
                    .upgrade()
                    .filter(|c| c.generation.get() == generation && c.widget.is_mapped())
                else {
                    return;
                };

                if let Ok(Some((total, samples))) = scan_res {
                    if total == 0 {
                        card.folder_summary.set_text("Empty folder");
                    } else if total == 1 {
                        card.folder_summary.set_text("1 item");
                    } else {
                        card.folder_summary.set_text(&format!("{total} items"));
                    }

                    for (idx, tile) in card.folder_cells.iter().enumerate() {
                        if let Some((name, is_dir, _child_path)) = samples.get(idx) {
                            tile.container.set_visible(true);
                            tile.label.set_text(name);
                            if *is_dir {
                                crate::assets::set_primary_icon(
                                    &tile.image,
                                    crate::assets::icons::FOLDER,
                                );
                            } else {
                                let icon_name = super::browser::icon_for_name(name);
                                crate::assets::set_primary_icon(&tile.image, icon_name);
                            }
                        } else {
                            tile.container.set_visible(false);
                        }
                    }
                }
            });
            self.task.replace(Some(task));
            return;
        }

        let policy = super::thumbnail::PreviewPolicy::current();
        if policy
            .for_streaming_preview()
            .check_entry(&entry, None)
            .is_err()
        {
            return;
        }
        let family = classify_by_name(&entry.native_name);
        let operation = operation(family);

        let task = glib::MainContext::default().spawn_local(async move {
            let cached = super::thumbnail::cached_preview_bytes(&path).await;
            let Some(card) = weak.upgrade().filter(|card| card.current(generation)) else {
                return;
            };
            if let Some(bytes) = cached
                && let Ok(texture) = gtk::gdk::Texture::from_bytes(&bytes)
            {
                crate::assets::remove_primary_icon(&card.image);
                card.image.set_pixel_size(104);
                card.image.set_paintable(Some(&texture));
            } else if cold_thumbnail(&entry) && policy.check_entry(&entry, None).is_ok() {
                super::thumbnail::set_thumbnail_or_icon(
                    &card.image,
                    &entry,
                    super::browser::entry_icon(&entry),
                    26,
                    104,
                );
            }
            drop(card);
            let Some(operation) = operation else {
                return;
            };
            let worker_cancellation = cancellation.clone();
            let result = glib::future_with_timeout(
                Duration::from_secs(3),
                gio::spawn_blocking(move || {
                    let source = std::fs::metadata(&path).ok()?;
                    policy
                        .for_streaming_preview()
                        .check(family, Some(source.len()))
                        .ok()?;
                    let result =
                        crate::sandbox::parse(&path, operation, 0, &worker_cancellation).ok()?;
                    let current = std::fs::metadata(&path).ok()?;
                    let identity = |metadata: &std::fs::Metadata| {
                        (
                            metadata.dev(),
                            metadata.ino(),
                            metadata.len(),
                            metadata.mtime(),
                            metadata.mtime_nsec(),
                        )
                    };
                    if worker_cancellation.is_cancelled()
                        || identity(&source) != identity(&current)
                        || result.data.len() > 65536
                    {
                        return None;
                    }
                    serde_json::from_slice::<Details>(&result.data).ok()
                }),
            )
            .await;
            if result.is_err() {
                cancellation.cancel();
            }
            if let Ok(Ok(Some(details))) = result
                && let Some(card) = weak.upgrade().filter(|card| card.current(generation))
            {
                card.apply_details(&details, family);
            }
        });
        self.task.replace(Some(task));
    }

    fn current(&self, generation: u64) -> bool {
        self.generation.get() == generation
            && self.widget.is_mapped()
            && super::theme::ThemeManager::shared().hover_previews()
            && self.entry.borrow().as_ref().is_some_and(|entry| {
                super::thumbnail::PreviewPolicy::current()
                    .for_streaming_preview()
                    .check_entry(entry, None)
                    .is_ok()
            })
    }

    fn apply_details(&self, details: &Details, family: FormatFamily) {
        if family == FormatFamily::Audio {
            if let Some(title) = details.title.as_deref() {
                self.title.set_text(title);
            }
            if let Some(artist) = details.artist.as_deref() {
                self.artist.set_text(artist);
            }
            if let Some(album) = details.album.as_deref() {
                self.album.set_text(album);
            }
        }
        let summary = detail_text(details, family);
        if !summary.is_empty() {
            self.details.set_text(&summary);
        }
    }

    fn cancel(&self) {
        self.generation.set(self.generation.get().wrapping_add(1));
        self.cancellation.borrow().cancel();
        if let Some(task) = self.task.borrow_mut().take() {
            task.abort();
        }
        super::thumbnail::cancel_thumbnail(&self.image);
    }
}

impl Drop for Card {
    fn drop(&mut self) {
        self.cancel();
    }
}

fn editing(widget: &gtk::Widget) -> bool {
    if widget.has_css_class("inline-rename") && widget.is_visible() {
        return true;
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        if editing(&widget) {
            return true;
        }
        child = widget.next_sibling();
    }
    false
}

fn label(class: &str, width: i32, lines: i32) -> gtk::Label {
    let label = gtk::Label::new(None);
    label.add_css_class(class);
    label.set_xalign(0.0);
    label.set_width_chars(width);
    label.set_max_width_chars(width);
    label.set_lines(lines);
    label.set_wrap(lines > 1);
    label.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label
}

fn operation(family: FormatFamily) -> Option<ParseOperation> {
    match family {
        FormatFamily::Image
        | FormatFamily::Svg
        | FormatFamily::Heif
        | FormatFamily::RawImage
        | FormatFamily::GeoTiff => Some(ParseOperation::ImageMetadata),
        FormatFamily::Audio | FormatFamily::Video => Some(ParseOperation::MediaMetadata),
        _ => None,
    }
}

fn cold_thumbnail(entry: &FileEntry) -> bool {
    classify_by_name(&entry.native_name) == FormatFamily::Audio
        || matches!(
            capabilities_by_name(&entry.native_name).thumbnail,
            Some(
                ThumbnailHandler::Image
                    | ThumbnailHandler::Heif
                    | ThumbnailHandler::RawImage
                    | ThumbnailHandler::Media
            )
        )
}

fn kind(entry: &FileEntry) -> &'static str {
    if entry.is_directory() {
        return "Folder";
    }
    match classify_by_name(&entry.native_name) {
        FormatFamily::Audio => "Song",
        FormatFamily::Video => "Video",
        FormatFamily::Image
        | FormatFamily::Svg
        | FormatFamily::Heif
        | FormatFamily::RawImage
        | FormatFamily::GeoTiff => "Photo",
        FormatFamily::Pdf => "PDF",
        FormatFamily::OfficeDocument => "Document",
        FormatFamily::Spreadsheet => "Spreadsheet",
        FormatFamily::PlainText => "Text",
        FormatFamily::Archive => "Archive",
        _ => "File",
    }
}

fn resolution_tag(w: u32, h: u32) -> &'static str {
    let max_dim = w.max(h);
    let min_dim = w.min(h);
    if max_dim >= 7680 || min_dim >= 4320 {
        "8K"
    } else if max_dim >= 3840 || min_dim >= 2160 {
        "4K UHD"
    } else if max_dim >= 2560 || min_dim >= 1440 {
        "1440p QHD"
    } else if max_dim >= 1920 || min_dim >= 1080 {
        "1080p FHD"
    } else if max_dim >= 1280 || min_dim >= 720 {
        "720p HD"
    } else {
        "SD"
    }
}

fn detail_text(details: &Details, family: FormatFamily) -> String {
    let mut values = Vec::new();
    if let Some(duration) = details
        .duration_seconds
        .filter(|value| value.is_finite() && *value >= 0.0 && *value < 315_360_000.0)
    {
        let seconds = duration.round() as u64;
        values.push(if seconds >= 3600 {
            format!(
                "{}:{:02}:{:02}",
                seconds / 3600,
                seconds % 3600 / 60,
                seconds % 60
            )
        } else {
            format!("{}:{:02}", seconds / 60, seconds % 60)
        });
    }
    if family != FormatFamily::Audio
        && let (Some(width), Some(height)) = (details.width, details.height)
        && width > 0
        && height > 0
    {
        let res_tag = resolution_tag(width, height);
        values.push(format!("{width} × {height} px ({res_tag})"));
    }
    if let Some(codec) = if family == FormatFamily::Audio {
        details.audio_codec.as_deref()
    } else {
        details.video_codec.as_deref()
    } {
        values.push(codec.to_uppercase());
    }
    values.join(" · ")
}

#[cfg(test)]
mod tests;
