// SPDX-License-Identifier: GPL-3.0-or-later

use super::{
    PDF_MAX_ZOOM, PDF_MIN_ZOOM, format_file_size, pdf_zoom_after_scroll,
    preview_width_for_empty_space,
};

pub(super) mod coordination_stream {
    use gtk::{glib, prelude::*, subclass::prelude::*};

    mod imp {
        use super::*;

        #[derive(Default)]
        pub struct Stream;

        #[glib::object_subclass]
        impl ObjectSubclass for Stream {
            const NAME: &'static str = "HermesPreviewCoordinationTestStream";
            type Type = super::Stream;
            type ParentType = gtk::MediaStream;
            type Interfaces = (gtk::gdk::Paintable,);
        }

        impl ObjectImpl for Stream {}
        impl MediaStreamImpl for Stream {
            fn play(&self) -> bool {
                true
            }

            fn pause(&self) {}

            fn seek(&self, timestamp: i64) {
                self.obj().update(timestamp);
                self.obj().seek_success();
            }

            fn update_audio(&self, _: bool, _: f64) {}
            fn realize(&self, _: gtk::gdk::Surface) {}
            fn unrealize(&self, _: gtk::gdk::Surface) {}
        }
        impl gtk::gdk::subclass::prelude::PaintableImpl for Stream {
            fn snapshot(&self, _: &gtk::gdk::Snapshot, _: f64, _: f64) {}
        }
    }

    glib::wrapper! {
        pub struct Stream(ObjectSubclass<imp::Stream>)
            @extends gtk::MediaStream, @implements gtk::gdk::Paintable;
    }
}

struct VideoFocusFixture {
    drawer: super::PreviewDrawer,
    music: coordination_stream::Stream,
    claims: std::rc::Rc<std::cell::Cell<usize>>,
}

impl VideoFocusFixture {
    fn new() -> Self {
        use crate::services::{LoadHandle, PreviewEvent, PreviewProvider, PreviewRequest};
        use gtk::{glib, prelude::*};
        use std::{cell::Cell, rc::Rc};

        struct Pending;
        impl PreviewProvider for Pending {
            fn load(&self, _: PreviewRequest, _: Rc<dyn Fn(PreviewEvent)>) -> LoadHandle {
                LoadHandle::new(|| {})
            }
        }

        let drawer = super::PreviewDrawer::new(Rc::new(Pending));
        let music: coordination_stream::Stream = glib::Object::new();
        music.stream_prepared(true, false, true, 60_000_000);
        let claims = Rc::new(Cell::new(0));
        let observed_claims = claims.clone();
        let weak_music = music.downgrade();
        drawer.set_audio_play_handler(Rc::new(move || {
            observed_claims.set(observed_claims.get() + 1);
            if let Some(music) = weak_music.upgrade() {
                music.pause();
            }
        }));
        let weak_state = Rc::downgrade(&drawer.state);
        music.connect_playing_notify(move |music| {
            if music.is_playing()
                && let Some(state) = weak_state.upgrade()
            {
                super::PreviewDrawer { state }.pause_audio_preview();
            }
        });
        Self {
            drawer,
            music,
            claims,
        }
    }

    fn video(&self, has_audio: bool) -> coordination_stream::Stream {
        use gtk::{glib, prelude::*};
        let stream: coordination_stream::Stream = glib::Object::new();
        stream.stream_prepared(has_audio, true, true, 60_000_000);
        self.drawer
            .state
            .render_video(super::media::MediaPreview::new_full(&stream));
        stream
    }
}

#[test]
#[ignore = "Requires isolated GTK display; fake streams only, no decoder, audio or source files"]
fn desktop_fake_video_focus_coordinates_both_directions_even_when_muted() {
    use gtk::prelude::*;
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let fixture = VideoFocusFixture::new();
    fixture.music.play();
    let video = fixture.video(true);
    video.set_muted(true);
    video.set_volume(0.0);
    video.notify("playing");
    assert!(fixture.music.is_playing());
    assert_eq!(fixture.claims.get(), 0);
    video.play();
    assert!(video.is_playing());
    assert!(!fixture.music.is_playing());
    assert_eq!(fixture.claims.get(), 1);
    video.notify("playing");
    video.update(1_000_000);
    video.set_volume(0.5);
    assert_eq!(fixture.claims.get(), 1);
    fixture.music.play();
    assert!(fixture.music.is_playing());
    assert!(!video.is_playing());
    assert_eq!(fixture.claims.get(), 1);
    video.play();
    assert!(video.is_playing());
    assert!(!fixture.music.is_playing());
    assert_eq!(fixture.claims.get(), 2);
    fixture.drawer.close();
    assert!(!fixture.music.is_playing());
}

#[test]
#[ignore = "Requires isolated GTK display; fake streams only, no decoder, audio or source files"]
fn desktop_fake_video_focus_waits_for_preparation_and_ignores_silent_video() {
    use gtk::{glib, prelude::*};
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    for has_audio in [true, false] {
        let fixture = VideoFocusFixture::new();
        fixture.music.play();
        let video: coordination_stream::Stream = glib::Object::new();
        fixture
            .drawer
            .state
            .render_video(super::media::MediaPreview::new_full(&video));
        assert!(fixture.music.is_playing());
        assert_eq!(fixture.claims.get(), 0);
        video.set_playing(true);
        assert!(video.is_playing());
        assert!(fixture.music.is_playing());
        assert_eq!(fixture.claims.get(), 0);
        video.stream_prepared(has_audio, true, true, 60_000_000);
        assert_eq!(fixture.claims.get(), usize::from(has_audio));
        assert_eq!(fixture.music.is_playing(), !has_audio);
        video.notify("prepared");
        video.notify("has-audio");
        video.notify("playing");
        assert_eq!(fixture.claims.get(), usize::from(has_audio));
        if !has_audio {
            fixture.drawer.pause_audio_preview();
            assert!(video.is_playing());
            fixture.music.pause();
            fixture.music.play();
            assert!(video.is_playing());
            assert_eq!(fixture.claims.get(), 0);
        }
        fixture.drawer.close();
        assert_eq!(fixture.music.is_playing(), !has_audio);
    }
    let fixture = VideoFocusFixture::new();
    let loading: coordination_stream::Stream = glib::Object::new();
    fixture
        .drawer
        .state
        .render_video(super::media::MediaPreview::new_full(&loading));
    loading.set_playing(true);
    fixture.music.play();
    assert!(!loading.is_playing());
    loading.stream_prepared(true, true, true, 60_000_000);
    assert!(fixture.music.is_playing());
    assert_eq!(fixture.claims.get(), 0);
}

#[test]
#[ignore = "Requires isolated GTK display; fake streams only, no decoder, audio or source files"]
fn desktop_fake_video_focus_never_resumes_music_and_releases_replaced_previews() {
    use crate::model::{EntryKind, FileEntry, FileEntryInner, Location, MetadataValue};
    use gtk::{glib, prelude::*};
    use std::rc::Rc;
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    for action in ["pause", "end", "error", "close", "replace"] {
        let fixture = VideoFocusFixture::new();
        fixture.music.play();
        let video = fixture.video(true);
        video.play();
        assert!(!fixture.music.is_playing());
        match action {
            "pause" => video.pause(),
            "end" => video.stream_ended(),
            "error" => video.set_error(glib::Error::new(
                gtk::gio::IOErrorEnum::Failed,
                "Synthetic error",
            )),
            "close" => fixture.drawer.close(),
            "replace" => fixture.drawer.show(FileEntry(Rc::new(FileEntryInner {
                location: Location::local("/generated-focus/pending.txt"),
                native_name: "pending.txt".into(),
                display_name: "pending.txt".into(),
                kind: EntryKind::File,
                size: MetadataValue::Unknown,
                modified_unix_seconds: MetadataValue::Unknown,
                music_tags: None,
            }))),
            _ => unreachable!(),
        }
        assert!(!fixture.music.is_playing(), "Music resumed after {action}");
        assert_eq!(fixture.claims.get(), 1);
        fixture.drawer.close();
        assert!(!video.is_playing());
        assert!(!fixture.music.is_playing());
        fixture.music.play();
        if matches!(action, "close" | "replace") {
            video.play();
            assert!(video.is_playing());
        }
        video.notify("playing");
        assert!(fixture.music.is_playing());
        assert_eq!(fixture.claims.get(), 1);
    }
    let fixture = VideoFocusFixture::new();
    let video = fixture.video(true);
    let weak_state = Rc::downgrade(&fixture.drawer.state);
    let claims = fixture.claims.clone();
    let music = fixture.music.clone();
    music.play();
    drop(fixture);
    assert!(weak_state.upgrade().is_none());
    video.play();
    assert!(music.is_playing());
    assert_eq!(claims.get(), 0);
}

#[test]
#[ignore = "Requires isolated GTK display and private preferences; no real media or decoding"]
fn desktop_preview_metadata_reformats_without_reloading_and_clears_recycled_values() {
    use std::{cell::Cell, rc::Rc};
    use gtk::prelude::*;
    use crate::{model::{EntryKind, FileEntry, FileEntryInner, Location, MetadataValue}, services::{LoadHandle, PreviewEvent, PreviewProvider, PreviewRequest}};
    struct Pending(Rc<Cell<usize>>);
    impl PreviewProvider for Pending {
        fn load(&self, _: PreviewRequest, _: Rc<dyn Fn(PreviewEvent)>) -> LoadHandle {
            self.0.set(self.0.get() + 1);
            LoadHandle::new(|| {})
        }
    }
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let manager = crate::ui::theme::ThemeManager::shared();
    manager.set_date_format("relative");
    manager.set_size_unit("decimal");
    let loads = Rc::new(Cell::new(0));
    let drawer = super::PreviewDrawer::new(Rc::new(Pending(loads.clone())));
    let timestamp = gtk::glib::DateTime::now_local().unwrap().to_unix() - 240;
    let entry = |name: &str, size, modified| FileEntry(Rc::new(FileEntryInner {
        location: Location::local(format!("/generated-formatting/{name}")),
        native_name: name.into(), display_name: name.into(), kind: EntryKind::File,
        size, modified_unix_seconds: modified, music_tags: None,
    }));
    let known = entry("known.txt", MetadataValue::Known(1_500_000), MetadataValue::Known(timestamp));
    drawer.show(known.clone());
    assert_eq!(loads.get(), 1);
    assert_eq!(drawer.state.size.text(), "1.5 MB");
    assert_eq!(drawer.state.modified.text(), "4m ago");
    manager.set_size_unit("binary");
    manager.set_date_format("iso");
    assert_eq!(drawer.state.size.text(), "1.4 MiB");
    assert_eq!(drawer.state.modified.text().as_str(), crate::ui::formatting::format_timestamp(timestamp));
    assert_eq!(loads.get(), 1);
    assert_eq!(drawer.state.current.borrow().as_ref(), Some(&known));
    assert!(drawer.state.modified.tooltip_text().unwrap().contains("("));
    drawer.show(entry("unknown.txt", MetadataValue::Unknown, MetadataValue::Unavailable));
    assert_eq!(loads.get(), 2);
    manager.set_date_format("relative");
    manager.set_size_unit("decimal");
    assert_eq!(drawer.state.size.text(), "—");
    assert_eq!(drawer.state.modified.text(), "—");
    assert!(drawer.state.modified.tooltip_text().is_none());
    drawer.close();
    manager.set_size_unit("binary");
    assert_eq!(drawer.state.size.text(), "—");
    drawer.show(known);
    assert_eq!(loads.get(), 3);
    assert_eq!(drawer.state.size.text(), "1.4 MiB");
    drawer.close();
}

#[test]
#[ignore = "Requires an isolated GTK display and private preferences; validates late preview policy changes"]
fn desktop_completed_previews_recheck_format_and_size_settings_before_rendering() {
    use gtk::prelude::*;
    use crate::{model::{EntryKind, FileEntry, FileEntryInner, Location, MetadataValue}, services::{LoadHandle, Preview, PreviewContent, PreviewProvider, PreviewRequest, PreviewRequestId}};
    struct Pending;
    impl PreviewProvider for Pending {
        fn load(&self, _: PreviewRequest, _: std::rc::Rc<dyn Fn(crate::services::PreviewEvent)>) -> LoadHandle {
            LoadHandle::new(|| {})
        }
    }
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let manager = crate::ui::theme::ThemeManager::shared();
    let drawer = super::PreviewDrawer::new(std::rc::Rc::new(Pending));
    let ready = |size| Preview {
        request_id: PreviewRequestId(1),
        entry: FileEntry(std::rc::Rc::new(FileEntryInner {
            music_tags: None,
            location: Location::local("/generated/never-decode.png"),
            native_name: "never-decode.png".into(),
            display_name: "never-decode.png".into(),
            kind: EntryKind::File,
            size: MetadataValue::Known(size),
            modified_unix_seconds: MetadataValue::Unknown,
        })),
        content_type: "image/png".into(),
        content: PreviewContent::Rasterized { png: b"not image data".to_vec() },
    };
    let feedback = || drawer.state.content.first_child().unwrap().last_child().and_downcast::<gtk::Label>().unwrap().text().to_string();
    manager.set_preview_images(false);
    drawer.state.render(ready(10));
    assert!(feedback().contains("disabled"));
    assert!(drawer.state.image.borrow().is_none());
    manager.set_preview_images(true);
    manager.set_max_preview_size_mb(1);
    drawer.state.render(ready(2_000_000));
    assert!(feedback().contains("maximum preview size"));
    assert!(drawer.state.image.borrow().is_none());
}

#[test]
fn formats_preview_file_sizes() {
    assert_eq!(format_file_size(999), "999 B");
    assert_eq!(format_file_size(1_200), "1.2 kB");
    assert_eq!(format_file_size(2_500_000), "2.5 MB");
}

#[test]
fn initial_preview_uses_all_of_the_unoccupied_width() {
    assert_eq!(preview_width_for_empty_space(2_000, 500), 1_500);
    assert_eq!(preview_width_for_empty_space(1_260, 510), 750);
    assert_eq!(preview_width_for_empty_space(1_260, 810), 450);
    assert_eq!(preview_width_for_empty_space(700, 650), 280);
}

#[test]
#[ignore = "Requires an isolated GTK display; preview split responds to columns and view changes"]
fn desktop_preview_fills_remaining_space_and_preserves_manual_divider_position() {
    use crate::services::{LoadHandle, PreviewEvent, PreviewProvider, PreviewRequest};
    use gtk::prelude::*;
    use std::{
        cell::Cell,
        rc::Rc,
        time::{Duration, Instant},
    };
    struct Pending;
    impl PreviewProvider for Pending {
        fn load(&self, _: PreviewRequest, _: Rc<dyn Fn(PreviewEvent)>) -> LoadHandle {
            LoadHandle::new(|| {})
        }
    }
    let pump = |window: &gtk::Window| {
        let deadline = Instant::now() + Duration::from_millis(450);
        while Instant::now() < deadline {
            while gtk::glib::MainContext::default().iteration(false) {}
            window.queue_draw();
            if let Some(clock) = window.frame_clock() {
                clock.emit_by_name::<()>("update", &[]);
                clock.emit_by_name::<()>("layout", &[]);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let drawer = super::PreviewDrawer::new(Rc::new(Pending));
    let split = gtk::Paned::new(gtk::Orientation::Horizontal);
    split.set_shrink_start_child(true);
    split.set_shrink_end_child(true);
    split.set_start_child(Some(&gtk::Box::new(gtk::Orientation::Horizontal, 0)));
    split.set_end_child(Some(&drawer.widget()));
    let occupied = Rc::new(Cell::new(520));
    let measured = occupied.clone();
    drawer.attach_split(&split, Rc::new(move || measured.get()));
    let window = gtk::Window::builder()
        .default_width(1_280)
        .default_height(700)
        .child(&split)
        .build();
    window.present();
    pump(&window);
    drawer.show(crate::model::FileEntry(Rc::new(
        crate::model::FileEntryInner {
            location: crate::model::Location::local("/generated-preview/image.png"),
            native_name: "image.png".into(),
            display_name: "image.png".into(),
            kind: crate::model::EntryKind::File,
            size: crate::model::MetadataValue::Unknown,
            modified_unix_seconds: crate::model::MetadataValue::Unknown,
            music_tags: None,
        },
    )));
    pump(&window);
    assert!((split.position() - 520).abs() <= 1);
    occupied.set(820);
    pump(&window);
    assert!((split.position() - 820.min(split.width() - super::MIN_WIDTH)).abs() <= 1);
    occupied.set(904);
    pump(&window);
    assert!((split.position() - 904.min(split.width() - super::MIN_WIDTH)).abs() <= 1);
    assert_eq!(drawer.state.last_occupied_width.get(), 904);
    let manual_position = split.position() - 50;
    split.set_position(manual_position);
    pump(&window);
    assert!(
        (split.position() - manual_position).abs() <= 1,
        "Manual divider position must remain stable"
    );
    drawer.close();
    window.close();
}

#[test]
fn pdf_scroll_zoom_stays_within_its_supported_range() {
    assert!(pdf_zoom_after_scroll(1.0, -1.0) > 1.0);
    assert!(pdf_zoom_after_scroll(2.0, 1.0) < 2.0);
    assert_eq!(pdf_zoom_after_scroll(PDF_MIN_ZOOM, 100.0), PDF_MIN_ZOOM);
    assert_eq!(pdf_zoom_after_scroll(PDF_MAX_ZOOM, -100.0), PDF_MAX_ZOOM);
}

#[test]
#[ignore = "Requires a desktop display; run explicitly"]
fn desktop_preview_preserves_request_lifecycle() {
    use crate::services::{
        LoadHandle, Preview, PreviewContent, PreviewEvent, PreviewProvider, PreviewRequest,
        PreviewRequestId,
    };
    use gtk::prelude::*;
    use std::{cell::Cell, rc::Rc};

    struct PendingProvider(Rc<Cell<bool>>);
    impl PreviewProvider for PendingProvider {
        fn load(&self, _: PreviewRequest, _: Rc<dyn Fn(PreviewEvent)>) -> LoadHandle {
            let cancelled = self.0.clone();
            LoadHandle::new(move || cancelled.set(true))
        }
    }
    gtk::init().expect("Desktop display available");
    crate::assets::prepare().expect("Icons load");
    let cancelled = Rc::new(Cell::new(false));
    let drawer = super::PreviewDrawer::new(Rc::new(PendingProvider(cancelled.clone())));
    let entry = crate::model::FileEntry(Rc::new(crate::model::FileEntryInner {
        location: crate::model::Location::local("/fixture/video.mp4"),
        native_name: "video.mp4".into(),
        display_name: "video.mp4".into(),
        kind: crate::model::EntryKind::File,
        size: crate::model::MetadataValue::Unknown,
        modified_unix_seconds: crate::model::MetadataValue::Unknown,
        music_tags: None,
    }));
    drawer.show(entry.clone());
    let request_id = drawer.state.current_request.get().unwrap();
    let deliver = |id| {
        drawer.state.handle_event(
            request_id,
            PreviewEvent::Ready(Preview {
                request_id: id,
                entry: entry.clone(),
                content_type: "text/plain".into(),
                content: PreviewContent::Text {
                    content: "Ready".into(),
                    truncated: false,
                },
            }),
        )
    };
    deliver(PreviewRequestId(request_id.0 + 1));
    assert!(
        drawer
            .state
            .content
            .first_child()
            .unwrap()
            .is::<gtk::Spinner>()
    );
    assert!(!cancelled.get());
    deliver(request_id);
    assert_eq!(drawer.state.current_request.get(), None);
    assert!(cancelled.get());
    let ready = drawer.state.content.first_child().unwrap();
    deliver(request_id);
    assert_eq!(drawer.state.content.first_child().unwrap(), ready);
    drawer.close();
    assert!(drawer.state.content.first_child().is_none());
    drawer.show(entry.clone());
    let second_request = drawer.state.current_request.get().unwrap();
    cancelled.set(false);
    drawer.close();
    assert!(cancelled.get());
    drawer.state.handle_event(
        second_request,
        PreviewEvent::Failed {
            request_id: second_request,
            entry,
            message: "Late error".into(),
        },
    );
    assert!(drawer.state.content.first_child().is_none());
}
