// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

use gtk::{gio, glib, prelude::*};
use sourceview5::prelude::*;

use crate::{
    model::{FileEntry, MetadataValue},
    sandbox_helper::geotiff::GeoTiffMetadata,
    services::{
        FormatFamily, LoadHandle, PdfTextLayer, Preview, PreviewContent, PreviewEvent,
        PreviewProvider, PreviewRequest, PreviewRequestId, classify_by_mime, classify_by_name,
    },
};

#[cfg(test)]
mod pdf_ranges_tests;
mod pdf_text;

const DEFAULT_WIDTH: i32 = 520;
const MIN_WIDTH: i32 = 280;
const MAX_WIDTH: i32 = 3_000;
const TEXT_BYTE_LIMIT: usize = 1024 * 1024;
const TRANSITION: Duration = Duration::from_millis(260);
const PDF_PAGE_GAP: i32 = 6;
const PDF_MIN_ZOOM: f64 = 1.0;
const PDF_MAX_ZOOM: f64 = 4.0;

struct PreviewState {
    provider: Rc<dyn PreviewProvider>,
    revealer: gtk::Revealer,
    pane: gtk::Box,
    title: gtk::Label,
    size: gtk::Label,
    modified: gtk::Label,
    content_type: gtk::Label,
    content: gtk::Box,
    media: RefCell<Option<gtk::Video>>,
    split: RefCell<Option<gtk::Paned>>,
    occupied_width: RefCell<Option<Rc<dyn Fn() -> i32>>>,
    current: RefCell<Option<FileEntry>>,
    load: RefCell<Option<LoadHandle>>,
    pdf_loads: Rc<RefCell<HashMap<i32, LoadHandle>>>,
    current_request: Cell<Option<PreviewRequestId>>,
    next_request: Cell<u64>,
    opened: Cell<bool>,
    last_split_width: Cell<i32>,
    animating: Cell<bool>,
    animation_generation: Rc<Cell<u64>>,
}

#[derive(Clone)]
pub struct PreviewDrawer {
    state: Rc<PreviewState>,
}

impl PreviewDrawer {
    pub fn new(provider: Rc<dyn PreviewProvider>) -> Self {
        let pane = gtk::Box::new(gtk::Orientation::Vertical, 0);
        pane.add_css_class("preview-pane");
        pane.set_size_request(MIN_WIDTH, -1);
        pane.set_hexpand(true);
        pane.set_vexpand(true);

        let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        header.add_css_class("preview-header");
        let icon = crate::assets::primary_icon(crate::assets::icons::DOCUMENTS, 18);
        let title = gtk::Label::new(None);
        title.add_css_class("preview-title");
        title.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        title.set_hexpand(true);
        title.set_xalign(0.0);
        let open = gtk::Button::builder()
            .tooltip_text("Open in default application")
            .valign(gtk::Align::Center)
            .build();
        open.set_child(Some(&crate::assets::primary_icon(
            crate::assets::icons::EXTERNAL_LINK,
            16,
        )));
        open.add_css_class("preview-header-action");
        let close = gtk::Button::builder()
            .tooltip_text("Close preview (Space)")
            .valign(gtk::Align::Center)
            .build();
        close.set_child(Some(&crate::assets::primary_icon(
            crate::assets::icons::X,
            16,
        )));
        close.add_css_class("preview-close");
        close.add_css_class("preview-header-action");
        header.append(&icon);
        header.append(&title);
        header.append(&open);
        header.append(&close);
        pane.append(&header);

        let metadata = gtk::Box::new(gtk::Orientation::Horizontal, 18);
        metadata.add_css_class("preview-metadata");
        let (size_group, size) = metadata_value("SIZE");
        let (modified_group, modified) = metadata_value("MODIFIED");
        let (type_group, content_type) = metadata_value("TYPE");
        metadata.append(&size_group);
        metadata.append(&modified_group);
        metadata.append(&type_group);
        pane.append(&metadata);

        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        content.add_css_class("preview-content");
        content.set_vexpand(true);
        pane.append(&content);

        let revealer = gtk::Revealer::builder()
            .child(&pane)
            .transition_duration(0)
            .transition_type(gtk::RevealerTransitionType::SlideLeft)
            .reveal_child(false)
            .build();

        let state = Rc::new(PreviewState {
            provider,
            revealer,
            pane,
            title,
            size,
            modified,
            content_type,
            content,
            media: RefCell::new(None),
            split: RefCell::new(None),
            occupied_width: RefCell::new(None),
            current: RefCell::new(None),
            load: RefCell::new(None),
            pdf_loads: Rc::new(RefCell::new(HashMap::new())),
            current_request: Cell::new(None),
            next_request: Cell::new(1),
            opened: Cell::new(false),
            last_split_width: Cell::new(0),
            animating: Cell::new(false),
            animation_generation: Rc::new(Cell::new(0)),
        });
        let weak = Rc::downgrade(&state);
        open.connect_clicked(move |_| {
            let Some(state) = weak.upgrade() else {
                return;
            };
            let location = state
                .current
                .borrow()
                .as_ref()
                .map(|entry| entry.location.clone());
            if let Some(location) = location {
                if let Some(stream) = state
                    .media
                    .borrow()
                    .as_ref()
                    .and_then(gtk::Video::media_stream)
                {
                    stream.set_playing(false);
                }
                super::browser::open_location(&location, &state.pane);
            }
        });
        let weak = Rc::downgrade(&state);
        close.connect_clicked(move |_| {
            if let Some(state) = weak.upgrade() {
                state.close();
            }
        });

        Self { state }
    }

    pub fn widget(&self) -> gtk::Widget {
        self.state.revealer.clone().upcast()
    }

    pub fn attach_split(&self, split: &gtk::Paned, occupied_width: Rc<dyn Fn() -> i32>) {
        self.state.split.replace(Some(split.clone()));
        self.state.occupied_width.replace(Some(occupied_width));
        let weak = Rc::downgrade(&self.state);
        split.add_tick_callback(move |split, _| {
            let Some(state) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let available = split.width();
            if available > 0
                && available != state.last_split_width.replace(available)
                && state.opened.get()
                && !state.animating.get()
            {
                let opening_width = state.opening_width(available);
                split.set_position(available.saturating_sub(opening_width));
            }
            glib::ControlFlow::Continue
        });
    }

    pub fn is_open(&self) -> bool {
        self.state.opened.get()
    }

    pub fn show(&self, entry: FileEntry) {
        self.state.show(entry);
    }

    pub fn close(&self) {
        self.state.close();
    }

    pub fn toggle(&self, entry: Option<FileEntry>) {
        if self.is_open() {
            self.close();
        } else if let Some(entry) = entry {
            self.show(entry);
        }
    }
}

impl PreviewState {
    fn show(self: &Rc<Self>, entry: FileEntry) {
        let was_open = self.opened.replace(true);
        let already_showing = self.current.borrow().as_ref() == Some(&entry);
        if !was_open {
            self.revealer.set_transition_duration(0);
            self.pane.set_size_request(0, -1);
            self.revealer.set_reveal_child(true);
            if let Some(split) = self.split.borrow().as_ref() {
                self.animate_open(split);
            }
        }
        if !was_open || !already_showing {
            self.load(entry, 0);
        }
    }

    fn animate_open(self: &Rc<Self>, split: &gtk::Paned) {
        let available = split.width();
        if available <= MIN_WIDTH {
            return;
        }
        self.last_split_width.set(available);
        let target = available.saturating_sub(self.opening_width(available));
        let start = available;
        split.set_position(start);
        let animation_id = self.animation_generation.get().saturating_add(1);
        self.animation_generation.set(animation_id);
        self.animating.set(true);

        if !super::motion::animations_enabled() {
            split.set_position(target);
            self.pane.set_size_request(MIN_WIDTH, -1);
            self.animating.set(false);
            return;
        }

        let started = Instant::now();
        let split = split.clone();
        let pane = self.pane.clone();
        let generation = self.animation_generation.clone();
        let weak = Rc::downgrade(self);
        let _tick = split.clone().add_tick_callback(move |_, _| {
            if generation.get() != animation_id {
                return glib::ControlFlow::Break;
            }
            let progress =
                (started.elapsed().as_secs_f64() / TRANSITION.as_secs_f64()).clamp(0.0, 1.0);
            let eased = super::motion::emphasized_deceleration(progress);
            let position = f64::from(start) + f64::from(target - start) * eased;
            split.set_position(position.round() as i32);
            if progress >= 1.0 {
                split.set_position(target);
                pane.set_size_request(MIN_WIDTH, -1);
                if let Some(state) = weak.upgrade() {
                    state.animating.set(false);
                }
                glib::ControlFlow::Break
            } else {
                glib::ControlFlow::Continue
            }
        });
    }

    fn opening_width(&self, available: i32) -> i32 {
        let occupied_width = self
            .occupied_width
            .borrow()
            .as_ref()
            .map_or(available.saturating_sub(DEFAULT_WIDTH), |width| width())
            .clamp(0, available);
        let desired_width = preview_width_for_empty_space(available, occupied_width);
        let maximum_width = MAX_WIDTH.min(available.saturating_sub(MIN_WIDTH).max(MIN_WIDTH));
        desired_width.clamp(MIN_WIDTH, maximum_width)
    }

    fn close(self: &Rc<Self>) {
        self.opened.set(false);
        self.animating.set(false);
        self.animation_generation
            .set(self.animation_generation.get().saturating_add(1));
        self.current_request.set(None);
        self.load.borrow_mut().take();
        self.pdf_loads.borrow_mut().clear();
        self.clear_content();
        self.revealer.set_transition_duration(0);
        self.revealer.set_reveal_child(false);
        if let Some(split) = self.split.borrow().as_ref() {
            split.set_position(split.width());
        }
        self.pane.set_size_request(MIN_WIDTH, -1);
    }

    fn load(self: &Rc<Self>, entry: FileEntry, pdf_page: i32) {
        self.current.replace(Some(entry.clone()));
        self.title.set_text(&entry.display_name);
        self.title
            .set_tooltip_text(Some(&entry.location.display_path()));
        self.size.set_text(&metadata_size(&entry));
        self.modified.set_text(&metadata_modified(&entry));
        self.content_type.set_text(file_extension(&entry));
        self.show_loading();
        self.load.borrow_mut().take();
        self.pdf_loads.borrow_mut().clear();

        let request_id = PreviewRequestId(self.next_request.get());
        self.next_request
            .set(self.next_request.get().saturating_add(1));
        self.current_request.set(Some(request_id));
        let weak = Rc::downgrade(self);
        let emit = Rc::new(move |event| {
            let Some(state) = weak.upgrade() else {
                return;
            };
            state.handle_event(request_id, event);
        });
        let load = self.provider.load(
            PreviewRequest {
                id: request_id,
                entry,
                text_byte_limit: TEXT_BYTE_LIMIT,
                pdf_page,
            },
            emit,
        );
        self.load.replace(Some(load));
    }

    fn handle_event(self: &Rc<Self>, expected: PreviewRequestId, event: PreviewEvent) {
        if self.current_request.get() != Some(expected) {
            return;
        }
        match event {
            PreviewEvent::Ready(preview) if preview.request_id == expected => {
                self.current_request.set(None);
                self.load.borrow_mut().take();
                self.render(preview);
            }
            PreviewEvent::Failed {
                request_id,
                entry,
                message,
            } if request_id == expected => {
                self.current_request.set(None);
                self.load.borrow_mut().take();
                self.title.set_text(&entry.display_name);
                self.show_message("Preview unavailable", &message);
            }
            PreviewEvent::Ready(_) | PreviewEvent::Failed { .. } => {}
        }
    }

    fn render(self: &Rc<Self>, preview: Preview) {
        let mut family = classify_by_mime(&preview.content_type);
        if family == FormatFamily::Unknown {
            if gio::content_type_is_a(&preview.content_type, "text/plain") {
                family = FormatFamily::PlainText;
            } else {
                family = classify_by_name(&preview.entry.native_name);
            }
        }
        self.content_type.set_text(family.display_label());
        self.clear_content();
        let is_audio = preview.content_type.starts_with("audio/");
        match preview.content {
            PreviewContent::Text { content, truncated } => {
                let buffer = sourceview5::Buffer::new(None);
                let languages = sourceview5::LanguageManager::default();
                let language = languages.guess_language(
                    preview.entry.location.native_path(),
                    Some(&preview.content_type),
                );
                buffer.set_language(language.as_ref());
                super::theme::register_source_buffer(&buffer);
                buffer.set_highlight_syntax(true);
                buffer.set_text(&content);
                let view = sourceview5::View::builder()
                    .buffer(&buffer)
                    .cursor_visible(false)
                    .editable(false)
                    .highlight_current_line(false)
                    .left_margin(14)
                    .right_margin(14)
                    .top_margin(12)
                    .bottom_margin(12)
                    .monospace(true)
                    .show_line_numbers(true)
                    .wrap_mode(gtk::WrapMode::None)
                    .build();
                view.add_css_class("preview-text");
                let scroll = gtk::ScrolledWindow::builder()
                    .child(&view)
                    .hscrollbar_policy(gtk::PolicyType::Automatic)
                    .vscrollbar_policy(gtk::PolicyType::Automatic)
                    .hexpand(true)
                    .vexpand(true)
                    .build();
                self.content.append(&scroll);
                if truncated {
                    let notice = gtk::Label::new(Some("Preview limited to the first 1 MB"));
                    notice.add_css_class("preview-note");
                    self.content.append(&notice);
                }
            }
            PreviewContent::Rasterized { png } => {
                let bytes = glib::Bytes::from_owned(png);
                match gtk::gdk::Texture::from_bytes(&bytes) {
                    Ok(texture) => {
                        let scroll = build_interactive_texture_view(&texture);
                        self.content.append(&scroll);
                    }
                    Err(error) => self.show_message("Preview unavailable", &error.to_string()),
                }
            }
            PreviewContent::GeoTiff { png, metadata } => {
                self.render_geotiff_viewer(png, metadata);
            }
            PreviewContent::SandboxedMedia { data } => {
                let bytes = glib::Bytes::from_owned(data);
                let stream = gio::MemoryInputStream::from_bytes(&bytes);
                let media = gtk::MediaFile::for_input_stream(&stream);
                let video = gtk::Video::for_media_stream(Some(&media));
                video.add_css_class("preview-media");
                video.set_autoplay(false);
                video.set_loop(false);
                video.set_hexpand(true);
                video.set_vexpand(true);
                self.media.replace(Some(video.clone()));
                self.content.append(&video);
                let media_kind = if is_audio { "audio" } else { "video" };
                let notice = gtk::Label::new(Some(&format!(
                    "Preview limited to the first 30 seconds. Open the file to play the full {media_kind}."
                )));
                notice.add_css_class("preview-note");
                notice.set_justify(gtk::Justification::Center);
                notice.set_wrap(true);
                notice.set_xalign(0.5);
                self.content.append(&notice);
            }
            PreviewContent::Image | PreviewContent::Media => {
                let hint = match family {
                    FormatFamily::Pdf => "PDF rendering requires Poppler to be installed",
                    FormatFamily::Video => "Video preview requires FFmpeg",
                    FormatFamily::Audio => "Audio preview requires FFmpeg",
                    _ => "The image could not be rendered by the sandboxed converter",
                };
                self.show_message("Preview unavailable", hint);
            }
            PreviewContent::Pdf {
                png,
                page,
                pages,
                text_layer,
            } => {
                self.render_pdf_viewer(preview.entry, png, page, pages, text_layer);
            }
            PreviewContent::Spreadsheet { table } => {
                let view = crate::ui::table_view::build_spreadsheet_view(&table);
                self.content.append(&view);
            }
            PreviewContent::AudioWaveform { png, metadata } => {
                self.render_audio_waveform(png, &metadata);
            }
            PreviewContent::Code { language, .. } => {
                self.show_message("Code Preview", &format!("SourceView5 placeholder for {}", language));
            }
            PreviewContent::Markdown { .. } => {
                self.show_message("Markdown Render", "Rich markdown viewer placeholder");
            }
            PreviewContent::Model3D { format, .. } => {
                self.show_message("3D Model Viewer", &format!("Canvas placeholder for {}", format));
            }
            PreviewContent::Unsupported => {
                self.show_message("No visual preview", family.unavailable_reason());
            }
        }
    }

    fn render_pdf_viewer(
        self: &Rc<Self>,
        entry: FileEntry,
        initial_png: Vec<u8>,
        initial_page: i32,
        pages: i32,
        initial_text_layer: Option<Arc<PdfTextLayer>>,
    ) {
        let page_count = pages.clamp(0, 10_000);
        let labels: Vec<_> = (1..=page_count).map(|page| page.to_string()).collect();
        let labels: Vec<_> = labels.iter().map(String::as_str).collect();
        let model = gtk::StringList::new(&labels);
        let selection = gtk::NoSelection::new(Some(model));
        let factory = gtk::SignalListItemFactory::new();
        let zoom = Rc::new(Cell::new(PDF_MIN_ZOOM));
        let page_width = Rc::new(Cell::new(0));
        let visible_pages = Rc::new(RefCell::new(HashMap::<
            i32,
            (gtk::Overlay, gtk::Picture, gtk::DrawingArea),
        >::new()));
        let text_layers = Rc::new(RefCell::new(HashMap::<i32, Arc<PdfTextLayer>>::new()));
        let pdf_ranges = Rc::new(RefCell::new(HashMap::<i32, (usize, usize)>::new()));
        let pdf_drag = Rc::new(Cell::new(PdfDrag::Idle));
        let pdf_anchor = Rc::new(Cell::new((-1i32, 0usize)));
        let pdf_granularity = Rc::new(Cell::new(1u8));
        let pdf_press = Rc::new(RefCell::new((
            Instant::now(),
            f64::MAX,
            f64::MAX,
            -1i32,
            0u8,
        )));
        if let Some(layer) = initial_text_layer {
            text_layers.borrow_mut().insert(initial_page, layer);
        }

        factory.connect_setup(|_, item| {
            let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
                return;
            };
            let overlay = gtk::Overlay::new();
            let picture = gtk::Picture::new();
            picture.set_can_shrink(true);
            picture.set_content_fit(gtk::ContentFit::Contain);
            picture.set_hexpand(true);
            picture.set_vexpand(true);
            let text_area = gtk::DrawingArea::new();
            text_area.set_hexpand(true);
            text_area.set_vexpand(true);
            text_area.set_accessible_role(gtk::AccessibleRole::Img);
            text_area.update_property(&[gtk::accessible::Property::Label("PDF page text")]);
            let spinner = gtk::Spinner::new();
            spinner.set_halign(gtk::Align::Center);
            spinner.set_valign(gtk::Align::Center);
            overlay.set_child(Some(&picture));
            overlay.add_overlay(&text_area);
            overlay.add_overlay(&spinner);
            overlay.set_hexpand(true);
            overlay.set_size_request(-1, 560);
            item.set_child(Some(&overlay));
        });

        let provider = self.provider.clone();
        let loads = self.pdf_loads.clone();
        let initial_page = Rc::new(RefCell::new(Some((initial_page, initial_png))));
        let next_request = Rc::new(Cell::new(self.next_request.get().saturating_add(10_000)));
        let entry_for_bind = entry.clone();
        let page_width_for_bind = page_width.clone();
        let visible_pages_for_bind = visible_pages.clone();
        let layers_for_bind = text_layers.clone();
        let ranges_for_bind = pdf_ranges.clone();
        factory.connect_bind(move |_, item| {
            let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
                return;
            };
            let page_index = item.position() as i32;
            let Some(overlay) = item.child().and_downcast::<gtk::Overlay>() else {
                return;
            };
            let Some(picture) = overlay.child().and_downcast::<gtk::Picture>() else {
                return;
            };
            let Some(spinner) = overlay.last_child().and_downcast::<gtk::Spinner>() else {
                return;
            };
            let Some(text_area) = picture.next_sibling().and_downcast::<gtk::DrawingArea>() else {
                return;
            };
            let binding_name = format!("pdf-page-{page_index}");
            overlay.set_widget_name(&binding_name);
            overlay.set_tooltip_text(None);
            let target_width = page_width_for_bind.get();
            overlay.set_size_request(if target_width > 0 { target_width } else { -1 }, 560);
            picture.set_paintable(gtk::gdk::Paintable::NONE);
            spinner.start();
            spinner.set_visible(true);
            let layers_for_draw = layers_for_bind.clone();
            let ranges_for_draw = ranges_for_bind.clone();
            text_area.set_draw_func(move |_, cr, width, height| {
                let Some(layer) = layers_for_draw.borrow().get(&page_index).cloned() else {
                    return;
                };
                let Some(&(start, end)) = ranges_for_draw.borrow().get(&page_index) else {
                    return;
                };
                if start == end {
                    return;
                }
                let Some(color) = pdf_selection_color() else {
                    return;
                };
                let (ox, oy, s) =
                    pdf_text::image_bounds(&layer, f64::from(width), f64::from(height));
                cr.set_source_rgba(
                    f64::from(color.red()),
                    f64::from(color.green()),
                    f64::from(color.blue()),
                    0.42,
                );
                for [x1, y1, x2, y2] in pdf_text::selection_runs(&layer, start, end) {
                    let (rx, ry) = (ox + f64::from(x1) * s, oy + f64::from(y1) * s);
                    let (rw, rh) = (f64::from(x2 - x1) * s, f64::from(y2 - y1) * s);
                    let pad = rh * 0.06;
                    rounded_rect(
                        cr,
                        rx - pad,
                        ry - pad,
                        rw + pad * 2.0,
                        rh + pad * 2.0,
                        (rh * 0.16).min(3.0),
                    );
                }
                let _ = cr.fill();
            });
            visible_pages_for_bind.borrow_mut().insert(
                page_index,
                (overlay.clone(), picture.clone(), text_area.clone()),
            );

            let is_initial_page = initial_page
                .borrow()
                .as_ref()
                .is_some_and(|(page, _)| *page == page_index);
            let cached = if is_initial_page {
                initial_page.borrow_mut().take()
            } else {
                None
            };
            if let Some((_, png)) = cached {
                set_pdf_page_texture(&overlay, &picture, png, page_width_for_bind.get());
                spinner.stop();
                spinner.set_visible(false);
                return;
            }

            let request_id = PreviewRequestId(next_request.get());
            next_request.set(next_request.get().saturating_add(1));
            let weak_overlay = overlay.downgrade();
            let weak_picture = picture.downgrade();
            let weak_spinner = spinner.downgrade();
            let weak_text_area = text_area.downgrade();
            let loads_for_event = loads.clone();
            let layers_for_event = layers_for_bind.clone();
            let page_width_for_event = page_width_for_bind.clone();
            let emit = Rc::new(move |event| {
                loads_for_event.borrow_mut().remove(&page_index);
                let Some(overlay) = weak_overlay
                    .upgrade()
                    .filter(|overlay| overlay.widget_name() == binding_name)
                else {
                    return;
                };
                match event {
                    PreviewEvent::Ready(Preview {
                        request_id: response_id,
                        content:
                            PreviewContent::Pdf {
                                png,
                                page,
                                text_layer,
                                ..
                            },
                        ..
                    }) if response_id == request_id && page == page_index => {
                        if let Some(layer) = text_layer {
                            layers_for_event.borrow_mut().insert(page_index, layer);
                            if let Some(area) = weak_text_area.upgrade() {
                                area.queue_draw();
                            }
                        }
                        if let Some(picture) = weak_picture.upgrade() {
                            set_pdf_page_texture(
                                &overlay,
                                &picture,
                                png,
                                page_width_for_event.get(),
                            );
                        }
                    }
                    PreviewEvent::Failed {
                        request_id: response_id,
                        ..
                    } if response_id == request_id => {
                        overlay.set_tooltip_text(Some("Unable to render this PDF page"));
                    }
                    PreviewEvent::Ready(_) | PreviewEvent::Failed { .. } => return,
                }
                if let Some(spinner) = weak_spinner.upgrade() {
                    spinner.stop();
                    spinner.set_visible(false);
                }
            });
            let load = provider.load(
                PreviewRequest {
                    id: request_id,
                    entry: entry_for_bind.clone(),
                    text_byte_limit: TEXT_BYTE_LIMIT,
                    pdf_page: page_index,
                },
                emit,
            );
            loads.borrow_mut().insert(page_index, load);
        });

        let loads = self.pdf_loads.clone();
        let visible_pages_for_unbind = visible_pages.clone();
        let layers_for_unbind = text_layers.clone();
        let ranges_for_unbind = pdf_ranges.clone();
        factory.connect_unbind(move |_, item| {
            if let Some(item) = item.downcast_ref::<gtk::ListItem>() {
                let page = item.position() as i32;
                loads.borrow_mut().remove(&page);
                visible_pages_for_unbind.borrow_mut().remove(&page);
                pdf_drop_unselected_layer(&layers_for_unbind, &ranges_for_unbind, page);
            }
        });

        let list = gtk::ListView::new(Some(selection), Some(factory));
        list.add_css_class("preview-pdf-list");
        list.set_hexpand(true);
        list.set_vexpand(true);
        let scroll = gtk::ScrolledWindow::builder()
            .child(&list)
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .vscrollbar_policy(gtk::PolicyType::Automatic)
            .hexpand(true)
            .vexpand(true)
            .build();

        scroll.add_css_class("preview-pdf-scroll");

        let zoom_scroll =
            gtk::EventControllerScroll::new(gtk::EventControllerScrollFlags::VERTICAL);
        zoom_scroll.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak_scroll = scroll.downgrade();
        let zoom_for_scroll = zoom.clone();
        let page_width_for_scroll = page_width.clone();
        let visible_pages_for_scroll = visible_pages.clone();
        zoom_scroll.connect_scroll(move |controller, _, dy| {
            if !controller
                .current_event_state()
                .contains(gtk::gdk::ModifierType::CONTROL_MASK)
            {
                return glib::Propagation::Proceed;
            }
            let Some(scroll) = weak_scroll.upgrade() else {
                return glib::Propagation::Stop;
            };
            let previous = zoom_for_scroll.get();
            let next = pdf_zoom_after_scroll(previous, dy);
            if (next - previous).abs() < f64::EPSILON {
                return glib::Propagation::Stop;
            }
            zoom_for_scroll.set(next);
            let width = pdf_page_width(&scroll, next);
            page_width_for_scroll.set(width);
            resize_pdf_pages(&visible_pages_for_scroll.borrow(), width);
            preserve_pdf_view_center(&scroll, next / previous);
            glib::Propagation::Stop
        });
        scroll.add_controller(zoom_scroll);

        let reset_zoom = gtk::EventControllerKey::new();
        reset_zoom.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak_scroll = scroll.downgrade();
        let zoom_for_reset = zoom.clone();
        let page_width_for_reset = page_width.clone();
        let visible_pages_for_reset = visible_pages.clone();
        reset_zoom.connect_key_pressed(move |_, key, _, modifiers| {
            if key.to_unicode() != Some('0')
                || !modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK)
            {
                return glib::Propagation::Proceed;
            }
            let Some(scroll) = weak_scroll.upgrade() else {
                return glib::Propagation::Stop;
            };
            zoom_for_reset.set(PDF_MIN_ZOOM);
            let width = pdf_page_width(&scroll, PDF_MIN_ZOOM);
            page_width_for_reset.set(width);
            resize_pdf_pages(&visible_pages_for_reset.borrow(), width);
            set_adjustment_value(&scroll.hadjustment(), 0.0);
            glib::Propagation::Stop
        });
        list.add_controller(reset_zoom);

        scroll.set_focusable(true);
        scroll.set_cursor_from_name(Some("grab"));
        let drag_origin = Rc::new(Cell::new((0.0, 0.0)));
        let pan = gtk::GestureDrag::new();
        pan.set_button(1);
        pan.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak_scroll = scroll.downgrade();
        let drag_origin_for_begin = drag_origin.clone();
        let pages_for_begin = visible_pages.clone();
        let layers_for_begin = text_layers.clone();
        let ranges_for_begin = pdf_ranges.clone();
        let drag_for_begin = pdf_drag.clone();
        let anchor_for_begin = pdf_anchor.clone();
        let granularity_for_begin = pdf_granularity.clone();
        let press_for_begin = pdf_press.clone();
        pan.connect_drag_begin(move |gesture, x, y| {
            let Some(scroll) = weak_scroll.upgrade() else {
                return;
            };
            let hit = pdf_page_at(
                &scroll,
                &pages_for_begin.borrow(),
                &layers_for_begin.borrow(),
                x,
                y,
            );
            let Some((page, _area, layer, px, py)) = hit else {
                drag_for_begin.set(PdfDrag::Pan);
                scroll.set_cursor_from_name(Some("grabbing"));
                scroll.grab_focus();
                drag_origin_for_begin
                    .set((scroll.hadjustment().value(), scroll.vadjustment().value()));
                anchor_for_begin.set((-1, 0));
                pdf_apply_ranges(
                    &ranges_for_begin,
                    &layers_for_begin,
                    &pages_for_begin.borrow(),
                    HashMap::new(),
                );
                return;
            };
            gesture.set_state(gtk::EventSequenceState::Claimed);
            drag_for_begin.set(PdfDrag::Select);
            scroll.grab_focus();
            let caret = pdf_text::caret_at(&layer, px, py);
            let shift = gesture
                .current_event_state()
                .contains(gtk::gdk::ModifierType::SHIFT_MASK);
            let mut press = press_for_begin.borrow_mut();
            let streak = if !shift
                && press.3 == page
                && press.0.elapsed() < Duration::from_millis(450)
                && (x - press.1).abs() <= 6.0
                && (y - press.2).abs() <= 6.0
            {
                (press.4 + 1).min(3)
            } else {
                1
            };
            *press = (Instant::now(), x, y, page, streak);
            drop(press);
            let extend = shift && anchor_for_begin.get().0 >= 0;
            granularity_for_begin.set(if extend { 1 } else { streak });
            let desired = if extend {
                pdf_desired_ranges(
                    &layers_for_begin.borrow(),
                    anchor_for_begin.get(),
                    (page, caret),
                    1,
                )
            } else {
                let unit = match streak {
                    2 => pdf_text::word_range(&layer, caret),
                    3 => pdf_text::line_range(&layer, caret),
                    _ => (caret, caret),
                };
                anchor_for_begin.set((page, unit.0));
                HashMap::from([(page, unit)])
                    .into_iter()
                    .filter(|(_, r)| r.0 != r.1)
                    .collect()
            };
            pdf_apply_ranges(
                &ranges_for_begin,
                &layers_for_begin,
                &pages_for_begin.borrow(),
                desired,
            );
        });

        let weak_scroll = scroll.downgrade();
        let pages_for_update = visible_pages.clone();
        let layers_for_update = text_layers.clone();
        let ranges_for_update = pdf_ranges.clone();
        let drag_for_update = pdf_drag.clone();
        let anchor_for_update = pdf_anchor.clone();
        let granularity_for_update = pdf_granularity.clone();
        pan.connect_drag_update(move |gesture, offset_x, offset_y| {
            let Some(scroll) = weak_scroll.upgrade() else {
                return;
            };
            if drag_for_update.get() == PdfDrag::Select {
                let Some((start_x, start_y)) = gesture.start_point() else {
                    return;
                };
                let pages = pages_for_update.borrow();
                let layers = layers_for_update.borrow();
                let Some((current_page, layer, px, py)) = pdf_page_near(
                    &scroll,
                    &pages,
                    &layers,
                    start_x + offset_x,
                    start_y + offset_y,
                ) else {
                    return;
                };
                let caret = pdf_text::caret_at(&layer, px, py);
                let desired = pdf_desired_ranges(
                    &layers,
                    anchor_for_update.get(),
                    (current_page, caret),
                    granularity_for_update.get(),
                );
                drop(layers);
                pdf_apply_ranges(&ranges_for_update, &layers_for_update, &pages, desired);
                return;
            }
            let (horizontal, vertical) = drag_origin.get();
            set_adjustment_value(&scroll.hadjustment(), horizontal - offset_x);
            set_adjustment_value(&scroll.vadjustment(), vertical - offset_y);
        });

        let weak_scroll = scroll.downgrade();
        let drag_for_end = pdf_drag.clone();
        pan.connect_drag_end(move |_, _, _| {
            let panned = drag_for_end.replace(PdfDrag::Idle) == PdfDrag::Pan;
            if panned && let Some(scroll) = weak_scroll.upgrade() {
                scroll.set_cursor_from_name(Some("grab"));
            }
        });
        scroll.add_controller(pan);

        let motion = gtk::EventControllerMotion::new();
        let weak_scroll = scroll.downgrade();
        let pages_for_motion = visible_pages.clone();
        let layers_for_motion = text_layers.clone();
        let drag_for_motion = pdf_drag.clone();
        motion.connect_motion(move |_, x, y| {
            if drag_for_motion.get() != PdfDrag::Idle {
                return;
            }
            let Some(scroll) = weak_scroll.upgrade() else {
                return;
            };
            let over_text = pdf_page_at(
                &scroll,
                &pages_for_motion.borrow(),
                &layers_for_motion.borrow(),
                x,
                y,
            )
            .is_some();
            scroll.set_cursor_from_name(Some(if over_text { "text" } else { "grab" }));
        });
        scroll.add_controller(motion);

        let keys = gtk::EventControllerKey::new();
        let weak_scroll = scroll.downgrade();
        let pages_for_keys = visible_pages.clone();
        let layers_for_keys = text_layers.clone();
        let ranges_for_keys = pdf_ranges.clone();
        keys.connect_key_pressed(move |_, key, _, modifiers| {
            if !pdf_shortcut_modifiers(modifiers) {
                return glib::Propagation::Proceed;
            }
            let Some(scroll) = weak_scroll.upgrade() else {
                return glib::Propagation::Proceed;
            };
            match key {
                gtk::gdk::Key::a | gtk::gdk::Key::A => {
                    let layers = layers_for_keys.borrow();
                    if layers.is_empty() {
                        return glib::Propagation::Proceed;
                    }
                    let desired = layers
                        .iter()
                        .map(|(page, layer)| (*page, (0, pdf_text::len(layer))))
                        .collect();
                    drop(layers);
                    pdf_apply_ranges(
                        &ranges_for_keys,
                        &layers_for_keys,
                        &pages_for_keys.borrow(),
                        desired,
                    );
                    glib::Propagation::Stop
                }
                gtk::gdk::Key::c | gtk::gdk::Key::C => {
                    let text =
                        pdf_selected_text(&layers_for_keys.borrow(), &ranges_for_keys.borrow());
                    if text.is_empty() {
                        return glib::Propagation::Proceed;
                    }
                    scroll.clipboard().set_text(&text);
                    glib::Propagation::Stop
                }
                _ => glib::Propagation::Proceed,
            }
        });
        scroll.add_controller(keys);

        let zoom_for_tick = zoom.clone();
        let page_width_for_tick = page_width.clone();
        let visible_pages_for_tick = visible_pages.clone();
        scroll.add_tick_callback(move |scroll, _| {
            if scroll.width() > PDF_PAGE_GAP * 2 + 1 {
                let width = pdf_page_width(scroll, zoom_for_tick.get());
                if width != page_width_for_tick.replace(width) {
                    resize_pdf_pages(&visible_pages_for_tick.borrow(), width);
                }
            }
            glib::ControlFlow::Continue
        });
        self.content.append(&scroll);
    }

    fn render_audio_waveform(
        &self,
        png: Vec<u8>,
        metadata: &crate::services::preview::AudioMetadata,
    ) {
        let container = gtk::Box::new(gtk::Orientation::Vertical, 12);
        container.set_vexpand(true);
        container.set_hexpand(true);
        container.set_margin_start(16);
        container.set_margin_end(16);
        container.set_margin_top(16);
        container.set_margin_bottom(16);

        let meta_box = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        meta_box.set_halign(gtk::Align::Center);

        let format_badge = gtk::Label::new(Some(&metadata.format));
        format_badge.add_css_class("badge");
        meta_box.append(&format_badge);

        let duration_mins = (metadata.duration_seconds / 60.0).floor() as i64;
        let duration_secs = (metadata.duration_seconds % 60.0).floor() as i64;
        let dur_label = gtk::Label::new(Some(&format!("{duration_mins:02}:{duration_secs:02}")));
        dur_label.add_css_class("dim-label");
        meta_box.append(&dur_label);

        let sr_khz = metadata.sample_rate as f64 / 1000.0;
        let ch_str = if metadata.channels == 1 {
            "Mono"
        } else if metadata.channels == 2 {
            "Stereo"
        } else {
            "Multi-ch"
        };
        let info_label = gtk::Label::new(Some(&format!("{sr_khz:.1} kHz • {ch_str}")));
        info_label.add_css_class("dim-label");
        meta_box.append(&info_label);

        container.append(&meta_box);

        let bytes = glib::Bytes::from_owned(png);
        if let Ok(texture) = gtk::gdk::Texture::from_bytes(&bytes) {
            let picture = gtk::Picture::for_paintable(&texture);
            picture.set_can_shrink(true);
            picture.set_content_fit(gtk::ContentFit::Contain);
            picture.set_hexpand(true);
            picture.set_vexpand(true);
            picture.set_halign(gtk::Align::Center);
            picture.set_valign(gtk::Align::Center);
            picture.set_size_request(-1, 240);
            container.append(&picture);
        }

        self.content.append(&container);
    }

    fn clear_content(&self) {
        if let Some(video) = self.media.borrow_mut().take()
            && let Some(stream) = video.media_stream()
        {
            stream.set_playing(false);
        }
        clear_box(&self.content);
    }

    fn show_loading(&self) {
        self.clear_content();
        let spinner = gtk::Spinner::new();
        spinner.add_css_class("preview-spinner");
        spinner.set_halign(gtk::Align::Center);
        spinner.set_valign(gtk::Align::Center);
        spinner.set_vexpand(true);
        spinner.start();
        self.content.append(&spinner);
    }

    fn show_message(&self, title: &str, detail: &str) {
        self.clear_content();
        let box_ = gtk::Box::new(gtk::Orientation::Vertical, 7);
        box_.add_css_class("preview-feedback");
        box_.set_halign(gtk::Align::Center);
        box_.set_valign(gtk::Align::Center);
        box_.set_vexpand(true);
        let heading = gtk::Label::new(Some(title));
        heading.add_css_class("preview-feedback-title");
        let detail = gtk::Label::new(Some(detail));
        detail.add_css_class("preview-feedback-detail");
        detail.set_justify(gtk::Justification::Center);
        detail.set_wrap(true);
        box_.append(&heading);
        box_.append(&detail);
        self.content.append(&box_);
    }

    fn render_geotiff_viewer(
        self: &Rc<Self>,
        png: Vec<u8>,
        metadata: GeoTiffMetadata,
    ) {
        let bytes = glib::Bytes::from_owned(png);
        let texture = match gtk::gdk::Texture::from_bytes(&bytes) {
            Ok(texture) => texture,
            Err(error) => {
                self.show_message("Preview unavailable", &error.to_string());
                return;
            }
        };

        let header_box = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        header_box.add_css_class("preview-gis-header");
        header_box.set_margin_start(16);
        header_box.set_margin_end(16);
        header_box.set_margin_top(12);
        header_box.set_margin_bottom(12);

        let badges = create_gis_badges(&metadata);
        badges.set_hexpand(true);
        let map_widget = create_placement_map(&metadata);
        map_widget.set_size_request(160, 90);
        map_widget.set_valign(gtk::Align::Center);

        header_box.append(&badges);
        header_box.append(&map_widget);
        self.content.append(&header_box);

        let scroll = build_interactive_texture_view(&texture);
        self.content.append(&scroll);
    }
}

fn create_gis_badges(metadata: &GeoTiffMetadata) -> gtk::Box {
    let vbox = gtk::Box::new(gtk::Orientation::Vertical, 8);
    vbox.set_hexpand(true);

    let row1 = gtk::Box::new(gtk::Orientation::Horizontal, 18);
    row1.set_hexpand(true);

    let (proj_group, proj_val) = metadata_value("PROJECTION");
    let proj_text = match (&metadata.crs_name, metadata.epsg) {
        (Some(crs), Some(epsg)) => format!("{crs} (EPSG:{epsg})"),
        (Some(crs), None) => crs.clone(),
        (None, Some(epsg)) => format!("EPSG:{epsg}"),
        (None, None) => "Unknown".to_string(),
    };
    proj_val.set_text(&proj_text);

    let (res_group, res_val) = metadata_value("RESOLUTION");
    res_val.set_text(&format!("{:.2} × {:.2}", metadata.resolution[0], metadata.resolution[1]));

    let (type_group, type_val) = metadata_value("DATA TYPE");
    let band_str = if metadata.band_count == 1 { "band" } else { "bands" };
    type_val.set_text(&format!("{} ({} {})", metadata.data_type, metadata.band_count, band_str));

    row1.append(&proj_group);
    row1.append(&res_group);
    row1.append(&type_group);

    let row2 = gtk::Box::new(gtk::Orientation::Horizontal, 18);
    row2.set_hexpand(true);

    let (bounds_group, bounds_val) = metadata_value("BOUNDS");
    let b = metadata.bounds;
    bounds_val.set_text(&format!("[{:.1}, {:.1}] – [{:.1}, {:.1}]", b[0], b[1], b[2], b[3]));

    let (dims_group, dims_val) = metadata_value("DIMENSIONS");
    dims_val.set_text(&format!("{} × {} px", metadata.dimensions[0], metadata.dimensions[1]));

    let (elev_group, elev_val) = metadata_value("ELEVATION RANGE");
    match (metadata.elevation_min, metadata.elevation_max) {
        (Some(min), Some(max)) => elev_val.set_text(&format!("{min:.1}m – {max:.1}m")),
        _ => elev_val.set_text("—"),
    }

    row2.append(&bounds_group);
    row2.append(&dims_group);
    row2.append(&elev_group);

    vbox.append(&row1);
    vbox.append(&row2);
    vbox
}

fn create_placement_map(metadata: &GeoTiffMetadata) -> gtk::DrawingArea {
    let area = gtk::DrawingArea::new();
    area.add_css_class("preview-placement-map");
    area.set_content_width(160);
    area.set_content_height(90);

    let bounds = metadata.bounds;
    let epsg = metadata.epsg;

    area.set_draw_func(move |_, context, width, height| {
        let w = f64::from(width);
        let h = f64::from(height);

        // 1. Clip rounded rectangle
        let r = 6.0;
        let degrees = std::f64::consts::PI / 180.0;
        context.new_sub_path();
        context.arc(w - r, r, r, -90.0 * degrees, 0.0 * degrees);
        context.arc(w - r, h - r, r, 0.0 * degrees, 90.0 * degrees);
        context.arc(r, h - r, r, 90.0 * degrees, 180.0 * degrees);
        context.arc(r, r, r, 180.0 * degrees, 270.0 * degrees);
        context.close_path();
        context.clip();

        // 2. Dark card fill
        context.set_source_rgba(0.10, 0.12, 0.16, 0.95);
        context.rectangle(0.0, 0.0, w, h);
        let _ = context.fill();

        // 3. Subtle card border
        context.set_source_rgba(1.0, 1.0, 1.0, 0.10);
        context.set_line_width(1.0);
        context.rectangle(0.5, 0.5, w - 1.0, h - 1.0);
        let _ = context.stroke();

        let pad_x = 8.0;
        let pad_y = 6.0;
        let map_w = w - pad_x * 2.0;
        let map_h = h - pad_y * 2.0;

        // 4. Graticule
        context.set_source_rgba(1.0, 1.0, 1.0, 0.08);
        context.set_line_width(0.8);
        // Equator
        context.move_to(pad_x, pad_y + map_h * 0.5);
        context.line_to(pad_x + map_w, pad_y + map_h * 0.5);
        // Prime Meridian
        context.move_to(pad_x + map_w * 0.5, pad_y);
        context.line_to(pad_x + map_w * 0.5, pad_y + map_h);
        let _ = context.stroke();

        // 5. Continents silhouettes
        draw_world_continents(context, pad_x, pad_y, map_w, map_h);

        // 6. Coordinate reprojection & footprint bounding box
        let (min_lon, min_lat, max_lon, max_lat) = normalize_to_lon_lat(bounds, epsg);
        let bx = pad_x + ((min_lon + 180.0) / 360.0) * map_w;
        let by = pad_y + ((90.0 - max_lat) / 180.0) * map_h;
        let bw = (((max_lon - min_lon) / 360.0) * map_w).max(4.0);
        let bh = (((max_lat - min_lat) / 180.0) * map_h).max(4.0);

        // Fill footprint
        context.set_source_rgba(0.20, 0.60, 1.00, 0.35);
        context.rectangle(bx, by, bw, bh);
        let _ = context.fill();

        // Stroke footprint
        context.set_source_rgba(0.30, 0.75, 1.00, 0.90);
        context.set_line_width(1.5);
        context.rectangle(bx, by, bw, bh);
        let _ = context.stroke();

        // Crosshair reticle if footprint is small
        if bw < 8.0 || bh < 8.0 {
            context.set_source_rgba(0.30, 0.75, 1.00, 0.60);
            context.set_line_width(0.75);
            let cx = bx + bw * 0.5;
            let cy = by + bh * 0.5;
            context.move_to(cx - 6.0, cy);
            context.line_to(cx + 6.0, cy);
            context.move_to(cx, cy - 6.0);
            context.line_to(cx, cy + 6.0);
            let _ = context.stroke();
        }
    });

    area
}

fn draw_world_continents(
    context: &gtk::cairo::Context,
    pad_x: f64,
    pad_y: f64,
    map_w: f64,
    map_h: f64,
) {
    let to_canvas = |lon: f64, lat: f64| -> (f64, f64) {
        let x = pad_x + ((lon + 180.0) / 360.0) * map_w;
        let y = pad_y + ((90.0 - lat) / 180.0) * map_h;
        (x, y)
    };

    let draw_poly = |pts: &[(f64, f64)]| {
        if pts.is_empty() {
            return;
        }
        let (x0, y0) = to_canvas(pts[0].0, pts[0].1);
        context.move_to(x0, y0);
        for pt in &pts[1..] {
            let (x, y) = to_canvas(pt.0, pt.1);
            context.line_to(x, y);
        }
        context.close_path();
    };

    // North America
    draw_poly(&[
        (-165.0, 65.0), (-140.0, 70.0), (-90.0, 70.0), (-60.0, 60.0),
        (-55.0, 45.0), (-80.0, 25.0), (-100.0, 20.0), (-110.0, 30.0),
        (-125.0, 50.0), (-165.0, 65.0),
    ]);

    // South America
    draw_poly(&[
        (-80.0, 10.0), (-35.0, -5.0), (-40.0, -22.0), (-55.0, -35.0),
        (-65.0, -55.0), (-75.0, -45.0), (-80.0, 0.0),
    ]);

    // Eurasia
    draw_poly(&[
        (-10.0, 36.0), (30.0, 36.0), (40.0, 30.0), (60.0, 25.0),
        (100.0, 10.0), (120.0, 20.0), (140.0, 40.0), (170.0, 65.0),
        (100.0, 75.0), (40.0, 70.0), (10.0, 55.0), (-10.0, 42.0),
    ]);

    // Africa
    draw_poly(&[
        (-15.0, 35.0), (35.0, 30.0), (50.0, 12.0), (42.0, -10.0),
        (30.0, -34.0), (18.0, -34.0), (10.0, 5.0), (-15.0, 15.0),
    ]);

    // Australia
    draw_poly(&[
        (115.0, -20.0), (150.0, -15.0), (150.0, -35.0),
        (135.0, -35.0), (115.0, -30.0),
    ]);

    context.set_source_rgba(1.0, 1.0, 1.0, 0.12);
    let _ = context.fill_preserve();
    context.set_source_rgba(1.0, 1.0, 1.0, 0.18);
    context.set_line_width(0.6);
    let _ = context.stroke();
}

fn normalize_to_lon_lat(bounds: [f64; 4], epsg: Option<u32>) -> (f64, f64, f64, f64) {
    let x_min = bounds[0].min(bounds[2]);
    let x_max = bounds[0].max(bounds[2]);
    let y_min = bounds[1].min(bounds[3]);
    let y_max = bounds[1].max(bounds[3]);

    let (lon_min, lat_min, lon_max, lat_max) = match epsg {
        Some(code @ 32601..=32660) | Some(code @ 32701..=32760) => {
            let is_south = code >= 32701;
            let zone = if is_south { code - 32700 } else { code - 32600 };
            let central_lon = (zone as f64) * 6.0 - 183.0;

            let northing_min = if is_south { y_min - 10_000_000.0 } else { y_min };
            let northing_max = if is_south { y_max - 10_000_000.0 } else { y_max };

            let lat1 = northing_min / 111_319.5;
            let lat2 = northing_max / 111_319.5;
            let avg_lat = ((lat1 + lat2) * 0.5).to_radians();
            let cos_lat = avg_lat.cos().abs().max(0.01);

            let lon1 = central_lon + (x_min - 500_000.0) / (111_319.5 * cos_lat);
            let lon2 = central_lon + (x_max - 500_000.0) / (111_319.5 * cos_lat);

            (lon1, lat1, lon2, lat2)
        }
        Some(3857) => {
            let lon1 = (x_min / 20_037_508.34) * 180.0;
            let lon2 = (x_max / 20_037_508.34) * 180.0;
            let lat1 = (180.0 / std::f64::consts::PI)
                * (2.0 * (y_min / 6_378_137.0).exp().atan() - std::f64::consts::FRAC_PI_2);
            let lat2 = (180.0 / std::f64::consts::PI)
                * (2.0 * (y_max / 6_378_137.0).exp().atan() - std::f64::consts::FRAC_PI_2);
            (lon1, lat1, lon2, lat2)
        }
        _ => {
            if x_min >= -180.0 && x_max <= 180.0 && y_min >= -90.0 && y_max <= 90.0 {
                (x_min, y_min, x_max, y_max)
            } else {
                (x_min.clamp(-180.0, 180.0), y_min.clamp(-90.0, 90.0), x_max.clamp(-180.0, 180.0), y_max.clamp(-90.0, 90.0))
            }
        }
    };

    let min_lon = lon_min.min(lon_max).clamp(-180.0, 180.0);
    let max_lon = lon_min.max(lon_max).clamp(-180.0, 180.0);
    let min_lat = lat_min.min(lat_max).clamp(-90.0, 90.0);
    let max_lat = lat_min.max(lat_max).clamp(-90.0, 90.0);

    (min_lon, min_lat, max_lon, max_lat)
}

fn build_interactive_texture_view(texture: &gtk::gdk::Texture) -> gtk::ScrolledWindow {
    let width = texture.width() as f64;
    let height = texture.height() as f64;

    let picture = gtk::Picture::for_paintable(texture);
    picture.add_css_class("preview-image");
    picture.set_can_shrink(true);
    picture.set_content_fit(gtk::ContentFit::Fill);
    picture.set_hexpand(true);
    picture.set_vexpand(true);
    picture.set_halign(gtk::Align::Center);
    picture.set_valign(gtk::Align::Center);

    let scroll = gtk::ScrolledWindow::builder()
        .child(&picture)
        .hscrollbar_policy(gtk::PolicyType::Automatic)
        .vscrollbar_policy(gtk::PolicyType::Automatic)
        .hexpand(true)
        .vexpand(true)
        .build();

    let zoom_level = Rc::new(Cell::new(1.0f64));
    let initial_fit_done = Rc::new(Cell::new(false));

    let vadj = scroll.vadjustment();
    let hadj = scroll.hadjustment();
    let pic = picture.clone();
    let zl = zoom_level.clone();
    let fit_done = initial_fit_done.clone();

    hadj.connect_notify_local(Some("page-size"), move |hadj, _| {
        if !fit_done.get() {
            let view_w = hadj.page_size();
            let view_h = vadj.page_size();
            if view_w > 0.0 && view_h > 0.0 {
                fit_done.set(true);
                let scale_w = view_w / width;
                let scale_h = view_h / height;
                let fit_scale = scale_w.min(scale_h).min(1.0);
                zl.set(fit_scale);
                pic.set_size_request(
                    (width * fit_scale) as i32,
                    (height * fit_scale) as i32,
                );
            }
        }
    });

    let pointer_pos = Rc::new(Cell::new((0.0, 0.0)));
    let motion = gtk::EventControllerMotion::new();
    let pointer_pos_clone = pointer_pos.clone();
    motion.connect_motion(move |_, x, y| {
        pointer_pos_clone.set((x, y));
    });
    scroll.add_controller(motion);

    let controller = gtk::EventControllerScroll::new(
        gtk::EventControllerScrollFlags::VERTICAL,
    );

    let picture_ref = picture.clone();
    let scroll_for_zoom = scroll.clone();
    controller.connect_scroll(move |_, _dx, dy| {
        let current = zoom_level.get();
        let factor = 1.15f64.powf(-dy);
        let new_zoom = (current * factor).clamp(0.1, 10.0);
        let actual_factor = new_zoom / current;

        zoom_level.set(new_zoom);

        picture_ref.set_size_request(
            (width * new_zoom) as i32,
            (height * new_zoom) as i32,
        );
        picture_ref.set_can_shrink(true);

        let (px, py) = pointer_pos.get();
        preserve_view_pointer(&scroll_for_zoom, actual_factor, px, py);

        gtk::glib::Propagation::Stop
    });

    scroll.add_controller(controller);

    let drag = gtk::GestureDrag::new();
    drag.set_button(1);
    let scroll_for_drag1 = scroll.clone();
    let start_x = Rc::new(Cell::new(0.0));
    let start_y = Rc::new(Cell::new(0.0));
    let start_x_clone = start_x.clone();
    let start_y_clone = start_y.clone();
    drag.connect_drag_begin(move |_, _, _| {
        let hadj = scroll_for_drag1.hadjustment();
        start_x_clone.set(hadj.value());
        let vadj = scroll_for_drag1.vadjustment();
        start_y_clone.set(vadj.value());
    });
    let scroll_for_drag2 = scroll.clone();
    drag.connect_drag_update(move |_, dx, dy| {
        let hadj = scroll_for_drag2.hadjustment();
        hadj.set_value(start_x.get() - dx);
        let vadj = scroll_for_drag2.vadjustment();
        vadj.set_value(start_y.get() - dy);
    });
    scroll.add_controller(drag);

    scroll
}

fn metadata_value(label: &str) -> (gtk::Box, gtk::Label) {
    let group = gtk::Box::new(gtk::Orientation::Vertical, 2);
    group.set_hexpand(true);
    group.set_valign(gtk::Align::Center);
    let heading = gtk::Label::new(Some(label));
    heading.add_css_class("preview-metadata-label");
    heading.set_xalign(0.0);
    let value = gtk::Label::new(Some("—"));
    value.add_css_class("preview-metadata-value");
    value.set_ellipsize(gtk::pango::EllipsizeMode::End);
    value.set_xalign(0.0);
    group.append(&heading);
    group.append(&value);
    (group, value)
}

fn set_pdf_page_texture(
    overlay: &gtk::Overlay,
    picture: &gtk::Picture,
    png: Vec<u8>,
    target_width: i32,
) {
    let Ok(texture) = gtk::gdk::Texture::from_bytes(&glib::Bytes::from_owned(png)) else {
        return;
    };
    picture.set_paintable(Some(&texture));
    resize_pdf_page(overlay, picture, target_width);
}

fn preview_width_for_empty_space(available: i32, occupied: i32) -> i32 {
    available
        .saturating_sub(occupied)
        .saturating_mul(9)
        .saturating_div(10)
        .max(MIN_WIDTH)
}

fn pdf_zoom_after_scroll(current: f64, dy: f64) -> f64 {
    (current * (-dy * 0.14).exp()).clamp(PDF_MIN_ZOOM, PDF_MAX_ZOOM)
}

fn pdf_page_width(scroll: &gtk::ScrolledWindow, zoom: f64) -> i32 {
    let fit_width = scroll.width().saturating_sub(PDF_PAGE_GAP * 2).max(1);
    (f64::from(fit_width) * zoom).round() as i32
}

fn resize_pdf_pages(
    pages: &HashMap<i32, (gtk::Overlay, gtk::Picture, gtk::DrawingArea)>,
    width: i32,
) {
    for (overlay, picture, _) in pages.values() {
        resize_pdf_page(overlay, picture, width);
    }
}

fn resize_pdf_page(overlay: &gtk::Overlay, picture: &gtk::Picture, target_width: i32) {
    let Some(paintable) = picture.paintable() else {
        overlay.set_size_request(if target_width > 0 { target_width } else { -1 }, 560);
        return;
    };
    let texture_width = paintable.intrinsic_width();
    let texture_height = paintable.intrinsic_height();
    let width = if target_width > 0 {
        target_width
    } else if overlay.width() > 1 {
        overlay.width()
    } else {
        return;
    };
    if texture_width > 0 && texture_height > 0 {
        let ratio = f64::from(texture_width) / f64::from(texture_height);
        overlay.set_size_request(width, (f64::from(width) / ratio).round() as i32);
    }
}

fn preserve_view_pointer(scroll: &gtk::ScrolledWindow, factor: f64, pointer_x: f64, pointer_y: f64) {
    let horizontal = scroll.hadjustment();
    let vertical = scroll.vadjustment();
    let old_x = horizontal.value() + pointer_x;
    let old_y = vertical.value() + pointer_y;
    glib::idle_add_local_once(glib::clone!(
        #[weak]
        scroll,
        move || {
            set_adjustment_value(
                &scroll.hadjustment(),
                old_x * factor - pointer_x,
            );
            set_adjustment_value(
                &scroll.vadjustment(),
                old_y * factor - pointer_y,
            );
        }
    ));
}

fn preserve_pdf_view_center(scroll: &gtk::ScrolledWindow, factor: f64) {
    let horizontal = scroll.hadjustment();
    let vertical = scroll.vadjustment();
    let horizontal_center = horizontal.value() + horizontal.page_size() / 2.0;
    let vertical_center = vertical.value() + vertical.page_size() / 2.0;
    glib::idle_add_local_once(glib::clone!(
        #[weak]
        scroll,
        move || {
            set_adjustment_value(
                &scroll.hadjustment(),
                horizontal_center * factor - scroll.hadjustment().page_size() / 2.0,
            );
            set_adjustment_value(
                &scroll.vadjustment(),
                vertical_center * factor - scroll.vadjustment().page_size() / 2.0,
            );
        }
    ));
}

fn set_adjustment_value(adjustment: &gtk::Adjustment, value: f64) {
    let maximum = (adjustment.upper() - adjustment.page_size()).max(adjustment.lower());
    adjustment.set_value(value.clamp(adjustment.lower(), maximum));
}

fn clear_box(box_: &gtk::Box) {
    while let Some(child) = box_.first_child() {
        box_.remove(&child);
    }
}

fn metadata_size(entry: &FileEntry) -> String {
    match entry.size {
        MetadataValue::Known(bytes) => format_file_size(bytes),
        MetadataValue::Unknown | MetadataValue::Unavailable => "—".to_owned(),
    }
}

fn metadata_modified(entry: &FileEntry) -> String {
    let MetadataValue::Known(seconds) = entry.modified_unix_seconds else {
        return "—".to_owned();
    };
    glib::DateTime::from_unix_local(seconds)
        .and_then(|date| date.format("%Y-%m-%d %H:%M"))
        .map(|value| value.to_string())
        .unwrap_or_else(|_| "—".to_owned())
}

fn file_extension(entry: &FileEntry) -> &str {
    entry
        .location
        .native_path()
        .and_then(|path| path.extension())
        .and_then(|extension| extension.to_str())
        .unwrap_or("file")
}

fn format_file_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "kB", "MB", "GB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000.0 && unit < UNITS.len() - 1 {
        value /= 1000.0;
        unit += 1;
    }
    if unit == 0 || value >= 10.0 {
        format!("{value:.0} {}", UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PdfDrag {
    Idle,
    Pan,
    Select,
}

fn pdf_page_at(
    scroll: &gtk::ScrolledWindow,
    pages: &HashMap<i32, (gtk::Overlay, gtk::Picture, gtk::DrawingArea)>,
    layers: &HashMap<i32, Arc<PdfTextLayer>>,
    x: f64,
    y: f64,
) -> Option<(i32, gtk::DrawingArea, Arc<PdfTextLayer>, f32, f32)> {
    for (page, (_, _, area)) in pages {
        let Some(layer) = layers.get(page) else {
            continue;
        };
        let Some(point) =
            scroll.compute_point(area, &gtk::graphene::Point::new(x as f32, y as f32))
        else {
            continue;
        };
        let (ox, oy, s) =
            pdf_text::image_bounds(layer, f64::from(area.width()), f64::from(area.height()));
        let px = (f64::from(point.x()) - ox) / s;
        let py = (f64::from(point.y()) - oy) / s;
        if px >= 0.0
            && py >= 0.0
            && px <= f64::from(layer.width)
            && py <= f64::from(layer.height)
            && pdf_text::hit_text(layer, px as f32, py as f32)
        {
            return Some((*page, area.clone(), layer.clone(), px as f32, py as f32));
        }
    }
    None
}

fn pdf_desired_ranges(
    layers: &HashMap<i32, Arc<PdfTextLayer>>,
    anchor: (i32, usize),
    caret: (i32, usize),
    granularity: u8,
) -> HashMap<i32, (usize, usize)> {
    let (anchor_page, anchor) = anchor;
    let (current_page, caret) = caret;
    let snap_lo = |layer: &PdfTextLayer, index: usize| match granularity {
        2 => pdf_text::word_range(layer, index).0,
        3 => pdf_text::line_range(layer, index).0,
        _ => index,
    };
    let snap_hi = |layer: &PdfTextLayer, index: usize| match granularity {
        2 => pdf_text::word_range(layer, index).1,
        3 => pdf_text::line_range(layer, index).1,
        _ => index,
    };
    let (first, last, first_start, last_end) = if anchor_page <= current_page {
        (anchor_page, current_page, anchor, caret)
    } else {
        (current_page, anchor_page, caret, anchor)
    };
    let mut desired = HashMap::new();
    for page in first..=last {
        let Some(layer) = layers.get(&page) else {
            continue;
        };
        let len = pdf_text::len(layer);
        let (start, end) = match (page == first, page == last) {
            (true, true) => (
                snap_lo(layer, first_start.min(last_end)),
                snap_hi(layer, first_start.max(last_end)),
            ),
            (true, false) => (snap_lo(layer, first_start), len),
            (false, true) => (0, snap_hi(layer, last_end)),
            _ => (0, len),
        };
        let (start, end) = (start.min(end), end.min(len));
        if start != end {
            desired.insert(page, (start, end));
        }
    }
    desired
}

fn pdf_drop_unselected_layer(
    layers: &RefCell<HashMap<i32, Arc<PdfTextLayer>>>,
    ranges: &RefCell<HashMap<i32, (usize, usize)>>,
    page: i32,
) {
    if !ranges.borrow().contains_key(&page) {
        layers.borrow_mut().remove(&page);
    }
}

fn pdf_apply_ranges(
    ranges: &RefCell<HashMap<i32, (usize, usize)>>,
    layers: &RefCell<HashMap<i32, Arc<PdfTextLayer>>>,
    pages: &HashMap<i32, (gtk::Overlay, gtk::Picture, gtk::DrawingArea)>,
    desired: HashMap<i32, (usize, usize)>,
) {
    let mut selected = ranges.borrow_mut();
    let mut dirty: Vec<i32> = desired
        .iter()
        .filter(|(page, range)| selected.get(*page) != Some(range))
        .map(|(page, _)| *page)
        .collect();
    dirty.extend(
        selected
            .keys()
            .filter(|page| !desired.contains_key(*page))
            .copied(),
    );
    *selected = desired;
    drop(selected);
    for page in &dirty {
        if !pages.contains_key(page) {
            pdf_drop_unselected_layer(layers, ranges, *page);
        }
    }
    for page in dirty {
        if let Some((_, _, area)) = pages.get(&page) {
            area.queue_draw();
        }
    }
}

fn pdf_page_near(
    scroll: &gtk::ScrolledWindow,
    pages: &HashMap<i32, (gtk::Overlay, gtk::Picture, gtk::DrawingArea)>,
    layers: &HashMap<i32, Arc<PdfTextLayer>>,
    x: f64,
    y: f64,
) -> Option<(i32, Arc<PdfTextLayer>, f32, f32)> {
    let mut nearest: Option<(i32, Arc<PdfTextLayer>, f32, f32, f64)> = None;
    for (page, (_, _, area)) in pages {
        let Some(layer) = layers.get(page) else {
            continue;
        };
        let Some(point) =
            scroll.compute_point(area, &gtk::graphene::Point::new(x as f32, y as f32))
        else {
            continue;
        };
        let (ox, oy, s) =
            pdf_text::image_bounds(layer, f64::from(area.width()), f64::from(area.height()));
        let (px, py) = (
            (f64::from(point.x()) - ox) / s,
            (f64::from(point.y()) - oy) / s,
        );
        let dx = px.clamp(0.0, f64::from(layer.width)) - px;
        let dy = py.clamp(0.0, f64::from(layer.height)) - py;
        let distance = dx * dx + dy * dy;
        let better = nearest.as_ref().is_none_or(|(.., best)| distance < *best);
        if better {
            nearest = Some((
                *page,
                layer.clone(),
                (px.clamp(0.0, f64::from(layer.width))) as f32,
                (py.clamp(0.0, f64::from(layer.height))) as f32,
                distance,
            ));
        }
    }
    nearest.map(|(page, layer, px, py, _)| (page, layer, px, py))
}

fn pdf_selected_text(
    layers: &HashMap<i32, Arc<PdfTextLayer>>,
    ranges: &HashMap<i32, (usize, usize)>,
) -> String {
    let mut pages: Vec<_> = ranges.iter().collect();
    pages.sort_by_key(|(page, _)| **page);
    let mut text = String::new();
    for (page, &(start, end)) in pages {
        let Some(layer) = layers.get(page) else {
            continue;
        };
        let part = pdf_text::selection_text(layer, start, end);
        if part.is_empty() {
            continue;
        }
        if !text.is_empty() && !text.ends_with('\n') {
            text.push('\n');
        }
        text.push_str(&part);
    }
    text
}

fn pdf_shortcut_modifiers(modifiers: gtk::gdk::ModifierType) -> bool {
    modifiers.contains(gtk::gdk::ModifierType::CONTROL_MASK)
        && !modifiers
            .intersects(gtk::gdk::ModifierType::SHIFT_MASK | gtk::gdk::ModifierType::ALT_MASK)
}

fn pdf_selection_color() -> Option<gtk::gdk::RGBA> {
    crate::ui::theme::ThemeManager::shared()
        .current_tokens()
        .and_then(|tokens| gtk::gdk::RGBA::parse(&tokens.accent).ok())
}

fn rounded_rect(cr: &gtk::cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    use std::f64::consts::{FRAC_PI_2, PI};
    let r = r.min(w / 2.0).min(h / 2.0);
    cr.new_sub_path();
    cr.arc(x + r, y + r, r, PI, 3.0 * FRAC_PI_2);
    cr.arc(x + w - r, y + r, r, 3.0 * FRAC_PI_2, 2.0 * PI);
    cr.arc(x + w - r, y + h - r, r, 0.0, FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, FRAC_PI_2, PI);
    cr.close_path();
}

#[cfg(test)]
mod tests;
