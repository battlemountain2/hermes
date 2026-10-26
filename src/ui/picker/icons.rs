use gtk::{gio, prelude::*};
use std::rc::Rc;

use super::State;

pub(super) fn watch_factory(factory: Option<gtk::ListItemFactory>, state: &Rc<State>) {
    let Some(factory) = factory.and_downcast::<gtk::SignalListItemFactory>() else {
        return;
    };
    let mut factories = state.factories.borrow_mut();
    factories.retain(|factory| factory.upgrade().is_some());
    if factories
        .iter()
        .any(|watched| watched.upgrade().as_ref() == Some(&factory))
    {
        return;
    }
    factories.push(factory.downgrade());
    drop(factories);
    let weak = Rc::downgrade(state);
    factory.connect_bind(move |_, item| {
        if let Some(state) = weak.upgrade().filter(|state| !state.closed.get())
            && item.find_property("child").is_some()
            && let Some(child) = item.property::<Option<gtk::Widget>>("child")
        {
            decorate_tree(&child, &state);
        }
    });
}

fn decorate_tree(widget: &gtk::Widget, state: &Rc<State>) {
    decorate(widget, state);
    let mut child = widget.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        decorate_tree(&widget, state);
    }
}

pub(super) fn decorate(widget: &gtk::Widget, state: &Rc<State>) {
    if widget.type_().name() == "GtkFileChooserCell"
        && widget.ancestor(gtk::GridView::static_type()).is_some()
    {
        widget.add_css_class("grid-card");
    }
    if let Some(label) = widget.downcast_ref::<gtk::Label>()
        && label.ancestor(gtk::GridView::static_type()).is_some()
    {
        label.add_css_class("grid-card-label");
    }
    if let Some(image) = widget.downcast_ref::<gtk::Image>()
        && image.has_css_class("sidebar-icon")
        && !image.has_css_class("picker-symbolic-icon")
    {
        image.add_css_class("picker-symbolic-icon");
        for property in ["gicon", "icon-name"] {
            image.connect_notify_local(Some(property), |image, _| apply_symbolic(image));
        }
        apply_symbolic(image);
    }
    if widget.type_().name() != "GtkFileThumbnail"
        || info_property(widget).is_none()
        || widget.find_property("icon-size").is_none()
    {
        return;
    }
    let Some(image) = widget.first_child().and_downcast::<gtk::Image>() else {
        return;
    };
    if image.has_css_class("picker-file-icon") {
        return;
    }
    image.add_css_class("picker-file-icon");
    let mut images = state.file_images.borrow_mut();
    images.retain(|image| image.upgrade().is_some());
    images.push(image.downgrade());
    drop(images);
    for property in [info_property(widget).unwrap(), "icon-size"] {
        let weak = image.downgrade();
        widget.connect_notify_local(Some(property), move |widget, _| {
            if let Some(image) = weak.upgrade() {
                refresh(widget, &image);
            }
        });
    }
    for property in ["gicon", "icon-name"] {
        let weak = widget.downgrade();
        image.connect_notify_local(Some(property), move |image, _| {
            if (image.gicon().is_some() || image.icon_name().is_some())
                && let Some(widget) = weak.upgrade()
            {
                refresh(&widget, image);
            }
        });
    }
    refresh(widget, &image);
}

fn refresh(widget: &gtk::Widget, image: &gtk::Image) {
    super::super::thumbnail::cancel_thumbnail(image);
    let grid = widget.property::<i32>("icon-size") > 32;
    image.set_size_request(if grid { 96 } else { 20 }, if grid { 88 } else { 20 });
    let Some(info) = widget.property::<Option<gio::FileInfo>>(info_property(widget).unwrap())
    else {
        image.clear();
        return;
    };
    let Some(file) = info
        .attribute_object("standard::file")
        .and_downcast::<gio::File>()
    else {
        crate::assets::set_primary_icon(image, crate::assets::icons::DOCUMENTS);
        return;
    };
    let Some(entry) = super::visuals::entry(&file, &info) else {
        return;
    };
    let icon_size = if grid { 26 } else { 18 };
    let fallback = super::super::browser::entry_icon(&entry);
    if super::super::thumbnail::PreviewPolicy::current()
        .check_entry(&entry, info.content_type().as_deref())
        .is_ok()
    {
        super::super::thumbnail::set_thumbnail_or_icon(
            image,
            &entry,
            fallback,
            icon_size,
            if grid { 80 } else { 20 },
        );
    } else {
        super::super::thumbnail::show_fallback_icon(image, fallback, icon_size);
    }
}

fn info_property(widget: &gtk::Widget) -> Option<&'static str> {
    ["file-info", "info"].into_iter().find(|name| {
        widget
            .find_property(name)
            .is_some_and(|property| property.value_type() == gio::FileInfo::static_type())
    })
}

fn apply_symbolic(image: &gtk::Image) {
    let names = image.icon_name().map(|name| vec![name]).or_else(|| {
        image
            .gicon()
            .and_downcast::<gio::ThemedIcon>()
            .map(|icon| icon.names())
    });
    let Some(icon) = names.as_ref().and_then(|names| {
        names.iter().find_map(|name| match name.as_str() {
            "user-home-symbolic" | "user-home" => Some(crate::assets::icons::HOME),
            "user-desktop-symbolic" | "folder-symbolic" | "folder" => {
                Some(crate::assets::icons::FOLDER)
            }
            "folder-documents-symbolic" => Some(crate::assets::icons::DOCUMENTS),
            "folder-download-symbolic" => Some(crate::assets::icons::DOWNLOADS),
            "folder-music-symbolic" => Some(crate::assets::icons::MUSIC),
            "folder-pictures-symbolic" => Some(crate::assets::icons::PICTURES),
            "folder-videos-symbolic" => Some(crate::assets::icons::VIDEOS),
            "document-open-recent-symbolic" => Some(crate::assets::icons::ROWS),
            "list-add-symbolic" => Some(crate::assets::icons::PLUS),
            "view-grid-symbolic" | "view-grid" => Some(crate::assets::icons::GRID),
            "view-list-symbolic" | "view-list" => Some(crate::assets::icons::LIST),
            "system-search-symbolic" => Some(crate::assets::icons::SEARCH),
            "folder-new-symbolic" => Some(crate::assets::icons::FOLDER),
            _ => None,
        })
    }) else {
        return;
    };
    crate::assets::set_primary_icon(image, icon);
    image.set_pixel_size(18);
}
