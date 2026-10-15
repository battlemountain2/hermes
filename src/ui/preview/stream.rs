use crate::playback::Session;
use gtk::{glib, prelude::*, subclass::prelude::*};
use std::{
    cell::{Cell, RefCell},
    path::Path,
    time::Duration,
};

mod imp {
    use super::*;
    #[derive(Default)]
    pub struct PlaybackStream {
        pub(super) session: RefCell<Option<Session>>,
        pub(super) texture: RefCell<Option<gtk::gdk::MemoryTexture>>,
        pub(super) poster: RefCell<Option<gtk::gdk::Texture>>,
        pub(super) stopped: Cell<bool>,
        pub(super) poster_cancellation: crate::sandbox::Cancellation,
    }
    #[glib::object_subclass]
    impl ObjectSubclass for PlaybackStream {
        const NAME: &'static str = "HermesPlaybackStream";
        type Type = super::PlaybackStream;
        type ParentType = gtk::MediaStream;
        type Interfaces = (gtk::gdk::Paintable,);
    }
    impl ObjectImpl for PlaybackStream {
        fn dispose(&self) {
            self.poster_cancellation.cancel();
            self.session.borrow_mut().take();
        }
    }
    impl MediaStreamImpl for PlaybackStream {
        fn play(&self) -> bool {
            self.session.borrow().as_ref().is_some_and(Session::play)
        }
        fn pause(&self) {
            if let Some(session) = self.session.borrow().as_ref() {
                session.pause();
            }
        }
        fn seek(&self, timestamp: i64) {
            if !self
                .session
                .borrow()
                .as_ref()
                .is_some_and(|session| session.seek(timestamp))
            {
                self.obj().seek_failed();
            }
        }
        fn update_audio(&self, muted: bool, volume: f64) {
            if let Some(session) = self.session.borrow().as_ref() {
                session.set_volume(muted, volume);
            }
        }
        fn realize(&self, _: gtk::gdk::Surface) {}
        fn unrealize(&self, _: gtk::gdk::Surface) {}
    }
    impl gtk::gdk::subclass::prelude::PaintableImpl for PlaybackStream {
        fn snapshot(&self, snapshot: &gtk::gdk::Snapshot, width: f64, height: f64) {
            if let Some(texture) = self.texture.borrow().as_ref() {
                texture.snapshot(snapshot, width, height);
            } else if let Some(poster) = self.poster.borrow().as_ref() {
                poster.snapshot(snapshot, width, height);
            }
        }
        fn intrinsic_width(&self) -> i32 {
            self.texture.borrow().as_ref().map_or_else(
                || {
                    self.poster
                        .borrow()
                        .as_ref()
                        .map_or(0, |texture| texture.width())
                },
                |texture| texture.width(),
            )
        }
        fn intrinsic_height(&self) -> i32 {
            self.texture.borrow().as_ref().map_or_else(
                || {
                    self.poster
                        .borrow()
                        .as_ref()
                        .map_or(0, |texture| texture.height())
                },
                |texture| texture.height(),
            )
        }
        fn intrinsic_aspect_ratio(&self) -> f64 {
            let width = self.intrinsic_width();
            let height = self.intrinsic_height();
            if height > 0 {
                f64::from(width) / f64::from(height)
            } else {
                0.0
            }
        }
    }
}

glib::wrapper! {
    pub struct PlaybackStream(ObjectSubclass<imp::PlaybackStream>)
        @extends gtk::MediaStream, @implements gtk::gdk::Paintable;
}

impl PlaybackStream {
    pub fn new(path: &Path) -> Self {
        Self::start(path, true)
    }

    pub fn new_audio(path: &Path) -> Self {
        Self::start(path, false)
    }

    fn start(path: &Path, with_poster: bool) -> Self {
        let stream: Self = glib::Object::new();
        let weak = stream.downgrade();
        let session_path = path.to_owned();
        glib::MainContext::default().spawn_local(async move {
            let result = gtk::gio::spawn_blocking(move || Session::start(&session_path)).await;
            let Some(stream) = weak.upgrade() else {
                return;
            };
            if stream.imp().stopped.get() {
                return;
            }
            match result {
                Ok(Ok(session)) => {
                    session.set_volume(stream.is_muted(), stream.volume());
                    stream.imp().session.replace(Some(session));
                }
                Ok(Err(message)) => {
                    stream.set_error(glib::Error::new(gtk::gio::IOErrorEnum::Failed, &message))
                }
                Err(_) => stream.set_error(glib::Error::new(
                    gtk::gio::IOErrorEnum::Failed,
                    "Playback startup failed",
                )),
            }
        });
        if with_poster {
            stream.reuse_cached_poster(path);
            let weak = stream.downgrade();
            let poster_path = path.to_owned();
            let cancellation = stream.imp().poster_cancellation.clone();
            glib::MainContext::default().spawn_local(async move {
                glib::timeout_future(Duration::from_millis(200)).await;
                if weak.upgrade().is_none_or(|stream| !stream.needs_poster()) {
                    return;
                }
                let result = gtk::gio::spawn_blocking(move || {
                    crate::sandbox::parse(
                        &poster_path,
                        crate::sandbox::ParseOperation::ThumbnailVideo,
                        256,
                        &cancellation,
                    )
                })
                .await;
                let Some(stream) = weak.upgrade() else {
                    return;
                };
                if !stream.needs_poster() {
                    return;
                }
                if let Ok(Ok(output)) = result
                    && let Ok(texture) =
                        gtk::gdk::Texture::from_bytes(&glib::Bytes::from_owned(output.data))
                {
                    stream.imp().poster.replace(Some(texture));
                    stream.invalidate_size();
                    stream.invalidate_contents();
                }
            });
        }
        let weak = stream.downgrade();
        glib::timeout_add_local(Duration::from_millis(16), move || {
            let Some(stream) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let updates = {
                let session = stream.imp().session.borrow();
                let Some(session) = session.as_ref() else {
                    return if stream.imp().stopped.get() || stream.error().is_some() {
                        glib::ControlFlow::Break
                    } else {
                        glib::ControlFlow::Continue
                    };
                };
                session.take_updates()
            };
            if let Some(message) = updates.error {
                stream.stop();
                stream.set_error(glib::Error::new(gtk::gio::IOErrorEnum::Failed, &message));
                return glib::ControlFlow::Break;
            }
            if let Some((duration, audio, video, seekable)) = updates.prepared {
                stream.stream_prepared(audio, video, seekable, duration);
            }
            if let Some((timestamp, width, height, data)) = updates.frame {
                stream.imp().poster_cancellation.cancel();
                stream.imp().poster.borrow_mut().take();
                let old_size = (stream.intrinsic_width(), stream.intrinsic_height());
                let texture = gtk::gdk::MemoryTexture::new(
                    width as i32,
                    height as i32,
                    gtk::gdk::MemoryFormat::R8g8b8a8,
                    &glib::Bytes::from_owned(data),
                    width as usize * 4,
                );
                stream.imp().texture.replace(Some(texture));
                if old_size != (width as i32, height as i32) {
                    stream.invalidate_size();
                }
                stream.invalidate_contents();
                if stream.is_prepared() && !stream.is_seeking() {
                    stream.update(timestamp);
                }
            }
            if let Some(timestamp) = updates.position {
                stream.update(timestamp);
            }
            if let Some(success) = updates.seeked {
                if success {
                    stream.seek_success();
                } else {
                    stream.seek_failed();
                }
            }
            if updates.ended {
                stream.stream_ended();
            }
            glib::ControlFlow::Continue
        });
        stream
    }

    fn needs_poster(&self) -> bool {
        !self.imp().stopped.get()
            && self.imp().texture.borrow().is_none()
            && self.imp().poster.borrow().is_none()
            && self.error().is_none()
            && !self.imp().poster_cancellation.is_cancelled()
    }

    fn reuse_cached_poster(&self, path: &Path) {
        let path = path.to_owned();
        let stream = self.downgrade();
        glib::MainContext::default().spawn_local(async move {
            let Some(bytes) = crate::ui::thumbnail::cached_preview_bytes(&path).await else { return; };
            let Some(stream) = stream.upgrade().filter(Self::needs_poster) else { return; };
            if let Ok(texture) = gtk::gdk::Texture::from_bytes(&bytes) {
                stream.imp().poster_cancellation.cancel();
                stream.imp().poster.replace(Some(texture));
                stream.invalidate_size();
                stream.invalidate_contents();
            }
        });
    }

    pub fn spectrum_bands(&self) -> [f32; crate::playback::SPECTRUM_BANDS] {
        self.imp()
            .session
            .borrow()
            .as_ref()
            .map_or([0.0; crate::playback::SPECTRUM_BANDS], |session| {
                session.spectrum_bands()
            })
    }

    pub fn stop(&self) {
        self.imp().stopped.set(true);
        self.imp().poster_cancellation.cancel();
        self.pause();
        self.imp().session.borrow_mut().take();
    }
}

#[cfg(test)]
mod tests;
