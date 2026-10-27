use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

use gtk::{gio, glib, prelude::*};

use crate::model::{EntryKind, FileEntry, FileEntryInner, Location, MetadataValue};

const ATTRIBUTES: &str =
    "standard::name,standard::display-name,standard::type,standard::size,standard::content-type";
type PreviewKey = (Option<String>, Option<String>);

pub(super) struct Visuals {
    pub widget: gtk::Box,
    image: gtk::Image,
    name: gtk::Label,
    size: gtk::Label,
    kind: gtk::Label,
    current: RefCell<Option<PreviewKey>>,
    generation: Cell<u64>,
    task: RefCell<Option<glib::JoinHandle<()>>>,
}

impl Visuals {
    pub fn new() -> Rc<Self> {
        let widget = gtk::Box::new(gtk::Orientation::Vertical, 0);
        widget.add_css_class("picker-preview");
        widget.add_css_class("preview-pane");
        widget.set_size_request(320, -1);
        widget.set_visible(false);
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        header.add_css_class("preview-header");
        header.append(&crate::assets::primary_icon(
            crate::assets::icons::DOCUMENTS,
            18,
        ));
        let name = gtk::Label::new(Some("Preview"));
        name.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        name.set_max_width_chars(28);
        name.set_hexpand(true);
        name.set_xalign(0.0);
        name.add_css_class("preview-title");
        header.append(&name);
        widget.append(&header);
        let metadata = gtk::Box::new(gtk::Orientation::Horizontal, 16);
        metadata.add_css_class("preview-metadata");
        let (size_group, size) = super::super::preview::metadata_value("SIZE");
        let (kind_group, kind) = super::super::preview::metadata_value("TYPE");
        metadata.append(&size_group);
        metadata.append(&kind_group);
        widget.append(&metadata);
        let stage = gtk::Box::new(gtk::Orientation::Vertical, 0);
        stage.add_css_class("picker-preview-stage");
        stage.set_vexpand(true);
        let image = gtk::Image::new();
        image.set_size_request(256, 224);
        image.set_vexpand(true);
        image.add_css_class("picker-preview-image");
        stage.append(&image);
        widget.append(&stage);
        Rc::new(Self {
            widget,
            image,
            name,
            size,
            kind,
            current: RefCell::new(None),
            generation: Cell::new(0),
            task: RefCell::new(None),
        })
    }

    pub fn update(self: &Rc<Self>, selected: Option<gio::File>, folder: Option<gio::File>) {
        let key = (
            selected.as_ref().map(|file| file.uri().to_string()),
            folder.as_ref().map(|file| file.uri().to_string()),
        );
        if self.current.borrow().as_ref() == Some(&key) {
            return;
        }
        self.current.replace(Some(key));
        self.cancel_work();
        super::super::thumbnail::show_fallback_icon(
            &self.image,
            crate::assets::icons::DOCUMENTS,
            48,
        );
        self.name.set_text("Preview");
        self.name.set_tooltip_text(None);
        self.size.set_text("—");
        self.kind.set_text("—");
        self.kind.set_tooltip_text(None);
        let Some(file) = selected else {
            self.widget.set_visible(false);
            return;
        };
        let generation = self.generation.get();
        let weak = Rc::downgrade(self);
        let task = glib::MainContext::default().spawn_local(async move {
            let info = metadata(&file).await;
            let Some(state) = weak
                .upgrade()
                .filter(|state| state.generation.get() == generation)
            else {
                return;
            };
            let Some(info) = info.filter(|info| info.file_type() != gio::FileType::Directory)
            else {
                state.widget.set_visible(false);
                return;
            };
            state.show_selected(&file, &info);
        });
        self.task.replace(Some(task));
    }

    fn show_selected(&self, file: &gio::File, info: &gio::FileInfo) {
        let Some(entry) = entry(file, info) else {
            return;
        };
        self.name.set_text(&entry.display_name);
        self.name.set_tooltip_text(Some(&entry.display_name));
        let description = info
            .content_type()
            .map(|mime| gio::content_type_get_description(&mime).to_string())
            .unwrap_or_else(|| "File".into());
        self.kind.set_text(&description);
        self.kind.set_tooltip_text(Some(&description));
        self.size
            .set_text(&glib::format_size(info.size().max(0) as u64));
        self.widget.set_visible(true);
        if super::super::thumbnail::PreviewPolicy::current()
            .check_entry(&entry, info.content_type().as_deref())
            .is_ok()
        {
            super::super::thumbnail::set_thumbnail_or_icon(
                &self.image,
                &entry,
                crate::assets::icons::DOCUMENTS,
                48,
                256,
            );
        }
    }

    pub fn cancel_work(&self) {
        self.generation.set(self.generation.get().wrapping_add(1));
        if let Some(task) = self.task.borrow_mut().take() {
            task.abort();
        }
        super::super::thumbnail::cancel_thumbnail(&self.image);
    }
}

impl Drop for Visuals {
    fn drop(&mut self) {
        self.cancel_work();
    }
}

async fn metadata(file: &gio::File) -> Option<gio::FileInfo> {
    let path = file.path().filter(|path| path.is_absolute())?;
    glib::future_with_timeout(
        Duration::from_secs(3),
        gio::File::for_path(path).query_info_future(
            ATTRIBUTES,
            gio::FileQueryInfoFlags::NONE,
            glib::Priority::DEFAULT_IDLE,
        ),
    )
    .await
    .ok()?
    .ok()
}

pub(super) fn entry(file: &gio::File, info: &gio::FileInfo) -> Option<FileEntry> {
    let path = file.path().filter(|path| path.is_absolute())?;
    Some(FileEntry(Rc::new(FileEntryInner {
        location: Location::local(&path),
        native_name: path.file_name()?.to_owned(),
        display_name: info.display_name().to_string(),
        kind: if info.file_type() == gio::FileType::Directory {
            EntryKind::Directory
        } else {
            EntryKind::File
        },
        size: MetadataValue::Known(info.size().max(0) as u64),
        modified_unix_seconds: MetadataValue::Unknown,
        music_tags: None,
    })))
}

#[cfg(test)]
mod tests;
