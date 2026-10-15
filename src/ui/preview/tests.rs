// SPDX-License-Identifier: GPL-3.0-or-later

use super::{
    PDF_MAX_ZOOM, PDF_MIN_ZOOM, format_file_size, pdf_zoom_after_scroll,
    preview_width_for_empty_space,
};

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
