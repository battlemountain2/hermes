use std::cell::RefCell;

use gtk::{gdk, glib, graphene, prelude::*, subclass::prelude::*};

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct ArtworkBackdrop {
        pub texture: RefCell<Option<gdk::Texture>>,
        pub base: RefCell<Option<gdk::RGBA>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for ArtworkBackdrop {
        const NAME: &'static str = "HermesArtworkBackdrop";
        type Type = super::ArtworkBackdrop;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for ArtworkBackdrop {}

    impl WidgetImpl for ArtworkBackdrop {
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let widget = self.obj();
            let bounds =
                graphene::Rect::new(0.0, 0.0, widget.width() as f32, widget.height() as f32);
            if let Some(base) = self.base.borrow().as_ref() {
                snapshot.append_color(base, &bounds);
            }
            if let Some(texture) = self.texture.borrow().as_ref() {
                snapshot.push_opacity(super::super::artwork::BACKDROP_OPACITY as f64);
                snapshot.append_texture(texture, &bounds);
                snapshot.pop();
            }
        }
    }
}

glib::wrapper! {
    pub struct ArtworkBackdrop(ObjectSubclass<imp::ArtworkBackdrop>)
        @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl ArtworkBackdrop {
    pub(super) fn new() -> Self {
        glib::Object::new()
    }

    pub(super) fn set_base(&self, color: gdk::RGBA) {
        if self.imp().base.borrow().as_ref() != Some(&color) {
            self.imp().base.replace(Some(color));
            self.queue_draw();
        }
    }

    pub(super) fn set_artwork(&self, texture: Option<gdk::Texture>) {
        if *self.imp().texture.borrow() != texture {
            self.imp().texture.replace(texture);
            self.queue_draw();
        }
    }
}

#[cfg(test)]
mod tests;
