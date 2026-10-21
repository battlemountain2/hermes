use super::*;
use crate::model::{EntryKind, FileEntryInner, Location};
use std::{fs, path::Path};

struct Pending(RefCell<Vec<(PreviewRequest, Rc<dyn Fn(PreviewEvent)>)>>);

impl PreviewProvider for Pending {
    fn load(&self, request: PreviewRequest, emit: Rc<dyn Fn(PreviewEvent)>) -> LoadHandle {
        self.0.borrow_mut().push((request, emit));
        LoadHandle::new(|| {})
    }
}

fn write_image(path: &Path, width: i32, height: i32, red: f64, blue: f64) -> Vec<u8> {
    let image =
        gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, width, height).unwrap();
    let drawing = gtk::cairo::Context::new(&image).unwrap();
    drawing.set_source_rgb(red, 0.0, blue);
    drawing.paint().unwrap();
    let mut bytes = Vec::new();
    image.write_to_png(&mut bytes).unwrap();
    fs::write(path, &bytes).unwrap();
    bytes
}

fn entry(path: &Path) -> FileEntry {
    FileEntry(Rc::new(FileEntryInner {
        location: Location::local(path),
        native_name: path.file_name().unwrap().to_owned(),
        display_name: path.file_name().unwrap().to_string_lossy().into_owned(),
        kind: EntryKind::File,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        music_tags: None,
    }))
}

fn pump_until(condition: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !condition() {
        assert!(
            Instant::now() < deadline,
            "Cached preview fixture timed out"
        );
        for _ in 0..64 {
            if !glib::MainContext::default().iteration(false) {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn pump() {
    let deadline = Instant::now() + Duration::from_millis(80);
    while Instant::now() < deadline {
        for _ in 0..64 {
            if !glib::MainContext::default().iteration(false) {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
#[ignore = "Private GTK display; generated cached photos/posters, no user files"]
fn desktop_cached_photos_upgrade_and_ignore_stale_changed_closed_or_failed_requests() {
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let root =
        std::env::temp_dir().join(format!("hermes-progressive-photo-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    let first = root.join("first.png");
    let second = root.join("second.png");
    let bytes = write_image(&first, 64, 32, 1.0, 0.0);
    crate::ui::thumbnail::tests::cache_fixture(&first, glib::Bytes::from_owned(bytes));
    let bytes = write_image(&second, 64, 32, 0.0, 1.0);
    crate::ui::thumbnail::tests::cache_fixture(&second, glib::Bytes::from_owned(bytes));
    let provider = Rc::new(Pending(RefCell::new(Vec::new())));
    let drawer = PreviewDrawer::new(provider.clone());
    let window = gtk::Window::builder()
        .child(&drawer.widget())
        .default_width(500)
        .default_height(500)
        .build();
    window.present();
    let cached_visible = || {
        drawer
            .state
            .content
            .first_child()
            .is_some_and(|child| child.has_css_class("preview-loading-image"))
    };
    let started = Instant::now();
    drawer.show(entry(&first));
    pump_until(cached_visible);
    let first_visible = started.elapsed();
    pump();
    assert!(drawer.state.loading_image.borrow().is_some());
    let loading_height = drawer
        .state
        .loading_image
        .borrow()
        .as_ref()
        .unwrap()
        .widget
        .first_child()
        .unwrap()
        .height();
    eprintln!(
        "Cached photo displayed while full decoder is pending: {:?}",
        first_visible
    );
    assert!(drawer.state.image.borrow().is_none());
    let (request, emit) = provider.0.borrow()[0].clone();
    let full = write_image(&root.join("full.png"), 400, 200, 1.0, 0.0);
    emit(PreviewEvent::Ready(Preview {
        request_id: request.id,
        entry: request.entry.clone(),
        content_type: "image/png".into(),
        content: PreviewContent::Rasterized { png: full.clone() },
    }));
    assert!(drawer.state.image.borrow().is_some());
    assert!(!cached_visible());
    pump();
    drawer.state.content.allocate(
        drawer.state.content.width(),
        drawer.state.content.height(),
        -1,
        None,
    );
    assert!(drawer.state.loading_image.borrow().is_none());
    assert_eq!(
        loading_height,
        drawer
            .state
            .image
            .borrow()
            .as_ref()
            .unwrap()
            .widget
            .first_child()
            .unwrap()
            .height()
    );
    drawer.show(entry(&second));
    pump_until(cached_visible);
    emit(PreviewEvent::Ready(Preview {
        request_id: request.id,
        entry: request.entry,
        content_type: "image/png".into(),
        content: PreviewContent::Rasterized { png: full.clone() },
    }));
    assert!(drawer.state.image.borrow().is_none());
    assert_eq!(drawer.state.title.text(), "second.png");
    let (request, emit) = provider.0.borrow()[1].clone();
    emit(PreviewEvent::Failed {
        request_id: request.id,
        entry: request.entry,
        message: "Generated decoder error".into(),
    });
    pump();
    assert!(!cached_visible());
    write_image(&first, 128, 64, 0.0, 1.0);
    drawer.show(entry(&first));
    pump();
    assert!(
        !cached_visible(),
        "Changed source must not show its old thumbnail"
    );
    let (request, emit) = provider.0.borrow()[2].clone();
    drawer.close();
    emit(PreviewEvent::Ready(Preview {
        request_id: request.id,
        entry: request.entry,
        content_type: "image/png".into(),
        content: PreviewContent::Rasterized { png: full },
    }));
    pump();
    assert!(drawer.state.image.borrow().is_none());
    assert!(!cached_visible());
    drawer.show(entry(&second));
    let (request, emit) = provider.0.borrow()[3].clone();
    emit(PreviewEvent::Ready(Preview {
        request_id: request.id,
        entry: request.entry,
        content_type: "image/png".into(),
        content: PreviewContent::Rasterized {
            png: write_image(&root.join("immediate.png"), 400, 200, 0.0, 1.0),
        },
    }));
    pump();
    assert!(drawer.state.image.borrow().is_some());
    assert!(
        !cached_visible(),
        "A late thumbnail must not replace a completed full preview"
    );
    drawer.close();
    window.close();
    fs::remove_dir_all(root).unwrap();
}
