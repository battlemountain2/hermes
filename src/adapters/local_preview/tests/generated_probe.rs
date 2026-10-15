use std::{
    cell::RefCell,
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};

use gtk::glib;

use crate::{
    model::{EntryKind, FileEntry, FileEntryInner, Location, MetadataValue},
    sandbox::{Cancellation, ParseOperation},
    services::{
        PreviewContent, PreviewEvent, PreviewProvider, PreviewRequest, PreviewRequestId,
        ThumbnailHandler,
    },
};

fn directory() -> PathBuf {
    let path = PathBuf::from(
        std::env::var_os("HERMES_PREVIEW_PROBE")
            .expect("Explicit generated fixture directory required"),
    );
    let path = path.canonicalize().unwrap();
    assert_eq!(path.parent(), Some(std::path::Path::new("/tmp")));
    assert!(
        path.file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("hermes-preview-audit.")
    );
    for variable in [
        "HOME",
        "XDG_CACHE_HOME",
        "XDG_CONFIG_HOME",
        "XDG_DATA_HOME",
        "XDG_STATE_HOME",
    ] {
        assert!(
            PathBuf::from(std::env::var_os(variable).expect("Private environment required"))
                .starts_with(&path)
        );
    }
    path
}

fn files(directory: &std::path::Path) -> Vec<String> {
    let mut files: Vec<String> =
        serde_json::from_slice(&std::fs::read(directory.join("manifest.json")).unwrap()).unwrap();
    if let Ok(filter) = std::env::var("HERMES_PROBE_FILTER") {
        files.retain(|name| name == &filter);
        assert!(!files.is_empty(), "Unknown fixture filter");
    }
    for name in &files {
        assert_eq!(std::path::Path::new(name).components().count(), 1);
    }
    files
}

fn entry(path: &std::path::Path) -> FileEntry {
    FileEntry(Rc::new(FileEntryInner {
        location: Location::local(path),
        native_name: path.file_name().unwrap().to_owned(),
        display_name: path.file_name().unwrap().to_string_lossy().into_owned(),
        kind: EntryKind::File,
        size: MetadataValue::Known(path.metadata().unwrap().len()),
        modified_unix_seconds: MetadataValue::Unknown,
        music_tags: None,
    }))
}

fn describe(event: &PreviewEvent) -> serde_json::Value {
    use serde_json::json;
    match event {
        PreviewEvent::Failed { message, .. } => json!({"error": message}),
        PreviewEvent::Ready(preview) => {
            let content = match &preview.content {
                PreviewContent::Rasterized { png } => {
                    json!({"kind": "raster", "bytes": png.len(), "dimensions": dimensions(png)})
                }
                PreviewContent::Text { content, truncated } => {
                    json!({"kind": "text", "bytes": content.len(), "text": content.chars().take(100).collect::<String>(), "truncated": truncated})
                }
                PreviewContent::Pdf {
                    png,
                    pages,
                    text_layer,
                    ..
                } => {
                    json!({"kind": "pdf", "bytes": png.len(), "pages": pages, "dimensions": dimensions(png), "text_layer": text_layer.is_some()})
                }
                PreviewContent::Audio { metadata } => {
                    json!({"kind": "audio-metadata", "properties": metadata.properties})
                }
                PreviewContent::StreamingMedia { .. } => {
                    json!({"kind": "native-path-only-not-decoded"})
                }
                PreviewContent::SandboxedAnimation { data } => {
                    json!({"kind": "animation-webm", "bytes": data.len()})
                }
                PreviewContent::Spreadsheet { table } => {
                    json!({"kind": "spreadsheet", "rows": table.total_rows, "cols": table.total_cols})
                }
                PreviewContent::GeoTiff { png, .. } => {
                    json!({"kind": "geotiff", "bytes": png.len(), "dimensions": dimensions(png)})
                }
                PreviewContent::Unsupported => json!({"kind": "unsupported"}),
                content => json!({"kind": format!("{content:?}")}),
            };
            json!({"mime": preview.content_type, "content": content})
        }
    }
}

fn dimensions(bytes: &[u8]) -> Option<[u32; 2]> {
    crate::sandbox_helper::sniff::sniff_png_dimensions(bytes).map(|size| [size.width, size.height])
}

fn thumbnail_operation(handler: ThumbnailHandler) -> ParseOperation {
    match handler {
        ThumbnailHandler::Image => ParseOperation::ThumbnailImage,
        ThumbnailHandler::Heif => ParseOperation::ThumbnailHeif,
        ThumbnailHandler::RawImage => ParseOperation::ThumbnailRaw,
        ThumbnailHandler::Pdf => ParseOperation::ThumbnailPdf,
        ThumbnailHandler::Media => ParseOperation::ThumbnailVideo,
        ThumbnailHandler::Model => ParseOperation::PreviewModel,
        ThumbnailHandler::Cover => ParseOperation::PreviewArchiveCover,
    }
}

#[test]
#[ignore = "Opt-in generated-fixture adapter/sandbox benchmark; private HOME/XDG and built helper required"]
fn generated_previewprobe_matrix() {
    let directory = directory();
    let context = glib::MainContext::default();
    let _guard = context.acquire().unwrap();
    let provider = super::super::LocalPreviewProvider;
    let mut rows = Vec::new();
    for name in files(&directory) {
        let path = directory.join("fixtures").join(&name);
        let capabilities = crate::services::capabilities_by_name(path.as_os_str());
        for sample in 0..3 {
            let captured = Rc::new(RefCell::new(None));
            let emitted = captured.clone();
            let start = Instant::now();
            let handle = provider.load(
                PreviewRequest {
                    id: PreviewRequestId(sample),
                    entry: entry(&path),
                    text_byte_limit: 1024 * 1024,
                    pdf_page: 0,
                },
                Rc::new(move |event| {
                    emitted.replace(Some(event));
                }),
            );
            let deadline = start + Duration::from_secs(35);
            while captured.borrow().is_none() && Instant::now() < deadline {
                for _ in 0..32 {
                    if !context.iteration(false) {
                        break;
                    }
                }
                std::thread::sleep(Duration::from_micros(100));
            }
            let event = captured
                .borrow_mut()
                .take()
                .expect("Adapter event deadline");
            if matches!(name.as_str(), "sample.docx" | "sample.odt" | "sample.zip") {
                assert!(
                    matches!(&event, PreviewEvent::Ready(preview)
                    if matches!(&preview.content, PreviewContent::Text { content, .. } if !content.is_empty())),
                    "Generated document/archive must show content, not a blank successful preview"
                );
            }
            let row = serde_json::json!({"file": name, "path": "local-adapter", "sample": sample, "ms": start.elapsed().as_secs_f64() * 1000.0, "family_by_name": format!("{:?}", capabilities.family), "outcome": describe(&event)});
            eprintln!("PROBE {row}");
            rows.push(row);
            drop(handle);
        }
        if let Some(handler) = capabilities.thumbnail {
            for sample in 0..3 {
                let start = Instant::now();
                let output = crate::sandbox::parse(
                    &path,
                    thumbnail_operation(handler),
                    128,
                    &Cancellation::default(),
                );
                let outcome = match output {
                    Ok(output) => {
                        serde_json::json!({"bytes": output.data.len(), "dimensions": dimensions(&output.data)})
                    }
                    Err(error) => serde_json::json!({"error": error}),
                };
                let row = serde_json::json!({"file": name, "path": "sandbox-thumbnail", "sample": sample, "ms": start.elapsed().as_secs_f64() * 1000.0, "outcome": outcome});
                eprintln!("PROBE {row}");
                rows.push(row);
            }
        }
    }
    let label = std::env::var("HERMES_PROBE_FILTER").unwrap_or_else(|_| "matrix".into());
    std::fs::write(
        directory.join(format!("adapter-{label}.json")),
        serde_json::to_vec_pretty(&rows).unwrap(),
    )
    .unwrap();
}

#[test]
#[ignore = "Opt-in actual native Session probe; generated files, silent paused preroll, private registry"]
fn generated_native_previewprobe_matrix() {
    let directory = directory();
    let mut rows = Vec::new();
    for name in files(&directory) {
        if !name.ends_with(".mp4") && !name.ends_with(".mov") {
            continue;
        }
        let path = directory.join("fixtures").join(&name);
        for sample in 0..3 {
            let start = Instant::now();
            let session = crate::playback::Session::start(&path).unwrap();
            session.set_volume(true, 0.0);
            let mut frame_ms = None;
            let mut prepared = None;
            let mut error = None;
            while start.elapsed() < Duration::from_secs(15) {
                let updates = session.take_updates();
                if updates.frame.is_some() && frame_ms.is_none() {
                    frame_ms = Some(start.elapsed().as_secs_f64() * 1000.0);
                }
                if updates.prepared.is_some() {
                    prepared = updates.prepared;
                }
                if updates.error.is_some() {
                    error = updates.error;
                    break;
                }
                if prepared.is_some() && frame_ms.is_some() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            let row = serde_json::json!({"file": name, "path": "native-session-paused-preroll", "sample": sample, "ms": start.elapsed().as_secs_f64() * 1000.0, "first_frame_ms": frame_ms, "prepared": prepared, "error": error});
            eprintln!("PROBE {row}");
            rows.push(row);
            drop(session);
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    std::fs::write(
        directory.join("native-matrix.json"),
        serde_json::to_vec_pretty(&rows).unwrap(),
    )
    .unwrap();
}

#[test]
#[ignore = "Opt-in generated-fixture cancellation/budget/replacement checks with actual sandbox"]
fn generated_previewprobe_cancellation_budgets_and_replacement() {
    let directory = directory();
    let fixtures = directory.join("fixtures");
    let cancelled = Cancellation::default();
    cancelled.cancel();
    for operation in [
        ParseOperation::PreviewImage,
        ParseOperation::ThumbnailImage,
        ParseOperation::PreviewAnimation,
        ParseOperation::PreviewPdf,
        ParseOperation::PreviewAudioMetadata,
        ParseOperation::PreviewOffice,
        ParseOperation::PreviewSpreadsheet,
    ] {
        let start = Instant::now();
        assert!(
            crate::sandbox::parse(&fixtures.join("sample.png"), operation, 0, &cancelled)
                .err()
                .unwrap()
                .contains("cancelled")
        );
        assert!(start.elapsed() < Duration::from_millis(100));
    }
    let token = Cancellation::default();
    let cancel = token.clone();
    let thread = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(30));
        cancel.cancel();
    });
    let start = Instant::now();
    let result = crate::sandbox::parse(
        &fixtures.join("sample.gif"),
        ParseOperation::PreviewAnimation,
        0,
        &token,
    );
    thread.join().unwrap();
    assert!(result.err().unwrap().contains("cancelled"));
    assert!(start.elapsed() < Duration::from_secs(1));
    eprintln!("In-flight animation cancellation: {:?}", start.elapsed());
    for (name, operation) in [
        ("corrupt-sample.jpg", ParseOperation::PreviewImage),
        ("corrupt-sample.png", ParseOperation::PreviewImage),
        ("corrupt-sample.webp", ParseOperation::PreviewImage),
        ("corrupt-sample.avif", ParseOperation::PreviewImage),
        ("corrupt-sample.heic", ParseOperation::PreviewHeif),
        ("corrupt-sample.gif", ParseOperation::PreviewAnimation),
        ("corrupt-sample.pdf", ParseOperation::PreviewPdf),
        ("corrupt-sample.mp3", ParseOperation::PreviewAudioMetadata),
        ("corrupt-sample.zip", ParseOperation::PreviewArchive),
        ("corrupt-sample.xlsx", ParseOperation::PreviewSpreadsheet),
        ("corrupt-sample.cbz", ParseOperation::PreviewArchiveCover),
        ("corrupt-sample.3mf", ParseOperation::PreviewModel),
        ("corrupt-sample.stl", ParseOperation::PreviewModel),
    ] {
        let error = crate::sandbox::parse(
            &fixtures.join(name),
            operation,
            1024 * 1024,
            &Cancellation::default(),
        )
        .err()
        .expect("Truncated fixture must be rejected");
        assert!(error.len() < 8192, "Unbounded decoder diagnostic");
        assert!(
            !error.contains("Incomplete browser header"),
            "{name}: {error}"
        );
    }
    let result = crate::sandbox::parse(
        &fixtures.join("large.png"),
        ParseOperation::PreviewImage,
        0,
        &Cancellation::default(),
    );
    assert!(result.err().unwrap().contains("decoded frame budget"));
    let context = glib::MainContext::default();
    let _guard = context.acquire().unwrap();
    let emitted = Rc::new(RefCell::new(false));
    let capture = emitted.clone();
    let handle = super::super::LocalPreviewProvider.load(
        PreviewRequest {
            id: PreviewRequestId(99),
            entry: entry(&fixtures.join("sample.gif")),
            text_byte_limit: 1024 * 1024,
            pdf_page: 0,
        },
        Rc::new(move |_| {
            capture.replace(true);
        }),
    );
    drop(handle);
    for _ in 0..50 {
        while context.iteration(false) {}
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(!*emitted.borrow());
    let replacement = fixtures.join("replacement-test.png");
    std::fs::copy(fixtures.join("sample.png"), &replacement).unwrap();
    let valid = crate::sandbox::parse(
        &replacement,
        ParseOperation::PreviewImage,
        0,
        &Cancellation::default(),
    )
    .unwrap();
    assert!(dimensions(&valid.data).is_some());
    std::fs::write(&replacement, b"corrupt replacement").unwrap();
    assert!(
        crate::sandbox::parse(
            &replacement,
            ParseOperation::PreviewImage,
            0,
            &Cancellation::default()
        )
        .is_err()
    );
    std::fs::remove_file(replacement).unwrap();
}

#[test]
#[ignore = "Opt-in actual 31-second cache revisit and generated decode/contention timing"]
fn generated_previewprobe_revisit_and_contention() {
    let directory = directory();
    let fixtures = directory.join("fixtures");
    let photo = fixtures.join("textured.jpg");
    let mut rows = Vec::new();
    for label in ["first-cache-miss", "immediate-hit", "after-31-seconds"] {
        if label == "after-31-seconds" {
            std::thread::sleep(Duration::from_secs(31));
        }
        let start = Instant::now();
        let output = crate::sandbox::parse(
            &photo,
            ParseOperation::PreviewImage,
            0,
            &Cancellation::default(),
        )
        .unwrap();
        let row = serde_json::json!({"path": "sandbox-selected-image", "label": label, "ms": start.elapsed().as_secs_f64() * 1000.0, "bytes": output.data.len()});
        eprintln!("PROBE {row}");
        rows.push(row);
    }
    for name in ["textured.jpg", "textured.heic", "sample.avif", "sample.svg"] {
        let path = fixtures.join(name);
        if !path.exists() {
            continue;
        }
        for sample in 0..3 {
            let start = Instant::now();
            let output = if name.ends_with(".heic") {
                crate::sandbox_helper::render_heif_preview(&path)
            } else {
                crate::sandbox_helper::render_image(&path, 1400)
            }
            .unwrap();
            let row = serde_json::json!({"file": name, "path": "direct-renderer-unsandboxed-control-only", "sample": sample, "ms": start.elapsed().as_secs_f64() * 1000.0, "bytes": output.len()});
            eprintln!("PROBE {row}");
            rows.push(row);
        }
    }
    for workers in [1, 2, 4] {
        crate::sandbox::browser::set_worker_limit(workers);
        for sample in 0..3 {
            let barrier = std::sync::Arc::new(std::sync::Barrier::new(5));
            let mut threads = Vec::new();
            for index in 0..4 {
                let copy = fixtures.join(format!("contention-{workers}-{sample}-{index}.jpg"));
                std::fs::copy(&photo, &copy).unwrap();
                let barrier = barrier.clone();
                threads.push(std::thread::spawn(move || {
                    barrier.wait();
                    let result = crate::sandbox::parse(
                        &copy,
                        ParseOperation::ThumbnailImage,
                        256,
                        &Cancellation::default(),
                    );
                    std::fs::remove_file(copy).unwrap();
                    result.unwrap().data.len()
                }));
            }
            barrier.wait();
            let start = Instant::now();
            let session = crate::playback::Session::start(&fixtures.join("hevc.mov")).unwrap();
            session.set_volume(true, 0.0);
            let mut first_frame = None;
            let mut error = None;
            while start.elapsed() < Duration::from_secs(15) {
                let updates = session.take_updates();
                if updates.frame.is_some() {
                    first_frame = Some(start.elapsed().as_secs_f64() * 1000.0);
                    break;
                }
                if updates.error.is_some() {
                    error = updates.error;
                    break;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            let row = serde_json::json!({"path": "native-hevc-with-four-cold-jpeg-thumbnails", "workers_per_pool": workers, "sample": sample, "first_frame_ms": first_frame, "error": error});
            eprintln!("PROBE {row}");
            rows.push(row);
            drop(session);
            for thread in threads {
                assert!(thread.join().unwrap() > 0);
            }
        }
    }
    crate::sandbox::browser::set_worker_limit(crate::sandbox::browser::default_worker_limit());
    std::fs::write(
        directory.join("revisit-contention.json"),
        serde_json::to_vec_pretty(&rows).unwrap(),
    )
    .unwrap();
}

struct PhaseProvider {
    start: Rc<RefCell<Instant>>,
    ready: Rc<RefCell<Option<f64>>>,
    render: Rc<RefCell<Option<f64>>>,
}

impl PreviewProvider for PhaseProvider {
    fn load(
        &self,
        request: PreviewRequest,
        emit: Rc<dyn Fn(PreviewEvent)>,
    ) -> crate::services::LoadHandle {
        let start = self.start.clone();
        let ready = self.ready.clone();
        let render = self.render.clone();
        super::super::LocalPreviewProvider.load(
            request,
            Rc::new(move |event| {
                assert!(matches!(event, PreviewEvent::Ready(_)), "{event:?}");
                ready.replace(Some(start.borrow().elapsed().as_secs_f64() * 1000.0));
                let rendering = Instant::now();
                emit(event);
                render.replace(Some(rendering.elapsed().as_secs_f64() * 1000.0));
            }),
        )
    }
}

fn find_native_stream(widget: &gtk::Widget) -> Option<crate::ui::preview::stream::PlaybackStream> {
    use gtk::prelude::*;
    if let Some(picture) = widget.downcast_ref::<gtk::Picture>() {
        if let Some(stream) = picture
            .paintable()
            .and_then(|paintable| paintable.downcast().ok())
        {
            return Some(stream);
        }
    }
    let mut child = widget.first_child();
    while let Some(current) = child {
        if let Some(stream) = find_native_stream(&current) {
            return Some(stream);
        }
        child = current.next_sibling();
    }
    None
}

#[test]
#[ignore = "Opt-in private connected Broadway display; real drawer fully-Ready/snapshot phases, not physical Wayland paint"]
fn generated_gtk_previewprobe_phases() {
    use gtk::prelude::*;
    let directory = directory();
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let context = glib::MainContext::default();
    let _guard = context.acquire().unwrap();
    let start = Rc::new(RefCell::new(Instant::now()));
    let ready = Rc::new(RefCell::new(None));
    let render = Rc::new(RefCell::new(None));
    let first_frame = Rc::new(RefCell::new(None));
    let painted = Rc::new(RefCell::new(None));
    let native = Rc::new(RefCell::new(false));
    let drawer = crate::ui::preview::PreviewDrawer::new(Rc::new(PhaseProvider {
        start: start.clone(),
        ready: ready.clone(),
        render: render.clone(),
    }));
    let widget = drawer.widget();
    let window = gtk::Window::builder()
        .default_width(900)
        .default_height(700)
        .child(&widget)
        .build();
    window.present();
    let settle = Instant::now();
    while settle.elapsed() < Duration::from_millis(300) {
        while context.iteration(false) {}
        std::thread::sleep(Duration::from_millis(1));
    }
    let clock = window.frame_clock().unwrap();
    let measured_widget = widget.clone();
    let measured_ready = ready.clone();
    let measured_frame = first_frame.clone();
    let measured_paint = painted.clone();
    let measured_start = start.clone();
    let measured_native = native.clone();
    let signal = clock.connect_after_paint(move |_| {
        if measured_ready.borrow().is_none() || measured_paint.borrow().is_some() {
            return;
        }
        if *measured_native.borrow() {
            let Some(stream) = find_native_stream(&measured_widget) else {
                return;
            };
            if stream.error().is_some() {
                panic!("{:?}", stream.error());
            }
            if !stream.is_prepared() || stream.intrinsic_width() <= 256 {
                return;
            }
            if measured_frame.borrow().is_none() {
                measured_frame.replace(Some(
                    measured_start.borrow().elapsed().as_secs_f64() * 1000.0,
                ));
            }
        }
        let snapshot = gtk::Snapshot::new();
        gtk::WidgetPaintable::new(Some(&measured_widget)).snapshot(&snapshot, 900.0, 700.0);
        if snapshot.to_node().is_some() {
            measured_paint.replace(Some(
                measured_start.borrow().elapsed().as_secs_f64() * 1000.0,
            ));
        }
    });
    let mut rows = Vec::new();
    for name in ["textured.jpg", "textured.heic", "h264.mov", "hevc.mov"] {
        for sample in 0..3 {
            drawer.close();
            let settle = Instant::now();
            while settle.elapsed() < Duration::from_millis(100) {
                while context.iteration(false) {}
                std::thread::sleep(Duration::from_millis(1));
            }
            ready.replace(None);
            render.replace(None);
            first_frame.replace(None);
            painted.replace(None);
            native.replace(name.ends_with(".mov"));
            start.replace(Instant::now());
            drawer.show(entry(&directory.join("fixtures").join(name)));
            clock.begin_updating();
            while painted.borrow().is_none() && start.borrow().elapsed() < Duration::from_secs(20) {
                for _ in 0..32 {
                    if !context.iteration(false) {
                        break;
                    }
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            clock.end_updating();
            let row = serde_json::json!({"file": name, "sample": sample, "path": "actual-drawer-private-broadway-after-paint-snapshot", "fully_ready_ms": *ready.borrow(), "synchronous_render_ms": *render.borrow(), "real_native_frame_observed_at_paint_ms": *first_frame.borrow(), "first_after_paint_snapshot_ms": *painted.borrow()});
            eprintln!("PROBE {row}");
            assert!(painted.borrow().is_some(), "No real after-paint snapshot");
            rows.push(row);
        }
    }
    clock.disconnect(signal);
    drawer.close();
    window.close();
    std::fs::write(
        directory.join("gtk-phases.json"),
        serde_json::to_vec_pretty(&rows).unwrap(),
    )
    .unwrap();
}
