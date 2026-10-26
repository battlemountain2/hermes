use super::*;

fn source() -> (PrivateOutput, Identity) {
    let directory = PrivateOutput::create().unwrap();
    let path = directory.path().join("fixture.docx");
    fs::write(&path, b"generated source").unwrap();
    let identity = Identity::read(&path).unwrap();
    (directory, identity)
}

#[test]
fn saved_posters_decode_without_a_conversion_or_office_profile() {
    let directory = PrivateOutput::create().unwrap();
    let path = directory.path().join("poster.pptx");
    let pixels = gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, 320, 180).unwrap();
    pixels.fill(0x305080ff);
    let mut archive = zip::ZipWriter::new(fs::File::create(&path).unwrap());
    for (name, bytes) in [
        ("ppt/presentation.xml", b"<presentation/>".to_vec()),
        (
            "docProps/thumbnail.png",
            pixels.save_to_bufferv("png", &[]).unwrap(),
        ),
    ] {
        archive
            .start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        archive.write_all(&bytes).unwrap();
    }
    archive.finish().unwrap();
    let bytes = crate::sandbox_helper::office::render::poster(&path, 160).unwrap();
    let dimensions = crate::sandbox_helper::sniff::sniff_png_dimensions(&bytes).unwrap();
    assert_eq!((dimensions.width, dimensions.height), (160, 90));
    assert!(!ParseOperation::OfficePoster.uses_office_renderer());
}

#[test]
fn converted_documents_are_reused_until_source_identity_or_ttl_changes() {
    let (_directory, identity) = source();
    let mut cache = Cache::default();
    let cancellation = Cancellation::default();
    let first = cache
        .document(&identity, &cancellation, || Ok(b"%PDF-generated".to_vec()))
        .unwrap();
    let cached = cache
        .document(&identity, &cancellation, || {
            panic!("Cache should avoid another conversion")
        })
        .unwrap();
    assert!(Arc::ptr_eq(&first, &cached));
    fs::write(&identity.path, b"changed generated document").unwrap();
    let changed = Identity::read(&identity.path).unwrap();
    let second = cache
        .document(&changed, &cancellation, || Ok(b"%PDF-second".to_vec()))
        .unwrap();
    assert!(!Arc::ptr_eq(&first, &second));
    assert_eq!(cache.0.len(), 1);
    cache.0[0].2 = Instant::now() - CACHE_TTL;
    let third = cache
        .document(&changed, &cancellation, || Ok(b"%PDF-third".to_vec()))
        .unwrap();
    assert!(!Arc::ptr_eq(&second, &third));
}

#[test]
fn cancelled_changed_failed_and_invalid_conversions_never_enter_the_cache() {
    let (_directory, identity) = source();
    let mut cache = Cache::default();
    let cancelled = Cancellation::default();
    cancelled.cancel();
    assert!(
        cache
            .document(&identity, &cancelled, || panic!("Cancelled"))
            .is_err()
    );
    let cancellation = Cancellation::default();
    assert!(
        cache
            .document(&identity, &cancellation, || Err("Conversion failed".into()))
            .is_err()
    );
    assert!(
        cache
            .document(&identity, &cancellation, || Ok(b"not a PDF".to_vec()))
            .is_err()
    );
    assert!(
        cache
            .document(&identity, &cancellation, || {
                fs::write(&identity.path, b"source replaced during conversion").unwrap();
                Ok(b"%PDF-stale".to_vec())
            })
            .is_err()
    );
    assert!(cache.0.is_empty());
    assert!(Document::new(vec![0; super::super::MAX_OUTPUT_BYTES as usize + 1]).is_err());
}

#[test]
fn conversion_cache_has_bounded_entries_and_preserves_active_readers() {
    let mut cache = Cache::default();
    let cancellation = Cancellation::default();
    let mut sources = Vec::new();
    let mut active = None;
    for _ in 0..CACHE_ENTRIES + 2 {
        let (directory, identity) = source();
        let document = cache
            .document(&identity, &cancellation, || Ok(b"%PDF-active".to_vec()))
            .unwrap();
        active.get_or_insert(document);
        sources.push(directory);
    }
    assert_eq!(cache.0.len(), CACHE_ENTRIES);
    assert_eq!(active.unwrap().bytes, b"%PDF-active");
}

#[test]
fn converted_pdf_memory_is_bounded_independently_of_entry_count() {
    let mut cache = Cache::default();
    let cancellation = Cancellation::default();
    let mut sources = Vec::new();
    for _ in 0..4 {
        let (directory, identity) = source();
        cache
            .document(&identity, &cancellation, || {
                let mut bytes = vec![0; 20 * 1024 * 1024];
                bytes[..5].copy_from_slice(b"%PDF-");
                Ok(bytes)
            })
            .unwrap();
        sources.push(directory);
    }
    assert_eq!(cache.0.len(), 3);
    assert!(
        cache
            .0
            .iter()
            .map(|(_, document, _)| document.bytes.len() as u64)
            .sum::<u64>()
            <= CACHE_BYTES
    );
}

#[test]
fn office_conversion_has_its_own_budgets_and_preserves_network_and_filesystem_isolation() {
    let command = super::super::sandbox_command(
        Path::new("/generated/helper"),
        Path::new("/private/source.docx"),
        Path::new("/private/output"),
        ParseOperation::ConvertOfficePdf,
        0,
    )
    .unwrap();
    let arguments: Vec<_> = command
        .get_args()
        .map(|value| value.to_string_lossy().into_owned())
        .collect();
    for required in [
        "--unshare-all",
        "--clearenv",
        "--die-with-parent",
        "--as=4294967296",
        "--data=536870912",
        "--cpu=30",
        "--fsize=33554432",
        "convert-office-pdf",
        "/input.docx",
        "/output/result.pdf",
    ] {
        assert!(
            arguments.iter().any(|value| value == required),
            "Missing {required}"
        );
    }
    assert!(!arguments.iter().any(|value| value == "/home"
        || value == "/run/user"
        || value.contains("DBUS_SESSION_BUS_ADDRESS")));
    let pdf = super::super::sandbox_command(
        Path::new("/generated/helper"),
        Path::new("/private/source.pdf"),
        Path::new("/private/output"),
        ParseOperation::PreviewPdf,
        0,
    )
    .unwrap();
    assert!(pdf.get_args().any(|value| value == "--as=1342177280"));
    assert!(
        !pdf.get_args()
            .any(|value| value.to_string_lossy().starts_with("--data="))
    );
}

#[test]
fn queued_office_work_can_be_cancelled_without_waiting_for_the_converter_lock() {
    let (_directory, identity) = source();
    let cache = Mutex::new(Cache::default());
    let _guard = cache.lock().unwrap();
    let cancellation = Cancellation::default();
    let worker_cancellation = cancellation.clone();
    let worker = thread::spawn(move || {
        thread::sleep(Duration::from_millis(30));
        worker_cancellation.cancel();
    });
    let started = Instant::now();
    assert!(
        render_cached(&identity.path, 0, &cancellation, &cache, |_| panic!(
            "No converter admission"
        ))
        .is_err()
    );
    assert!(started.elapsed() < Duration::from_secs(1));
    worker.join().unwrap();
}

#[test]
fn conversion_receives_the_identity_checked_canonical_source_not_a_retargeted_symlink() {
    let (directory, identity) = source();
    let alternate = directory.path().join("alternate.docx");
    fs::write(&alternate, b"alternate generated source").unwrap();
    let alias = directory.path().join("alias.docx");
    std::os::unix::fs::symlink(&identity.path, &alias).unwrap();
    let cache = Mutex::new(Cache::default());
    let cancellation = Cancellation::default();
    assert!(
        render_cached(&alias, 0, &cancellation, &cache, |resolved| {
            assert_eq!(resolved, identity.path);
            fs::remove_file(&alias).unwrap();
            std::os::unix::fs::symlink(&alternate, &alias).unwrap();
            assert_eq!(fs::read(resolved).unwrap(), b"generated source");
            cancellation.cancel();
            Ok(b"%PDF-cancelled".to_vec())
        })
        .is_err()
    );
    assert!(cache.lock().unwrap().0.is_empty());
}

#[test]
#[ignore = "Requires bubblewrap and coreutils; generated harmless child tree, no Office process or audio"]
fn admitted_office_conversion_cancellation_terminates_its_sandbox_process_tree() {
    let directory = PrivateOutput::create().unwrap();
    let helper = super::super::helper_executable().unwrap();
    let base = super::super::sandbox_command(
        &helper,
        Path::new("/dev/null"),
        directory.path(),
        ParseOperation::ConvertOfficePdf,
        0,
    )
    .unwrap();
    let arguments: Vec<_> = base.get_args().map(ToOwned::to_owned).collect();
    let separator = arguments
        .iter()
        .rposition(|argument| argument == "--")
        .unwrap();
    let mut command = std::process::Command::new(base.get_program());
    command.args(&arguments[..=separator]).args([
        "/usr/bin/sh",
        "-c",
        "touch /output/started; sleep 30",
    ]);
    let marker = directory.path().join("started");
    let cancellation = Cancellation::default();
    let worker_cancellation = cancellation.clone();
    let worker = thread::spawn(move || {
        let until = Instant::now() + Duration::from_secs(2);
        while !marker.exists() && Instant::now() < until {
            thread::sleep(Duration::from_millis(5));
        }
        let admitted = marker.exists();
        worker_cancellation.cancel();
        admitted
    });
    let started = Instant::now();
    let result = super::super::run_one_shot(
        command,
        &directory,
        ParseOperation::ConvertOfficePdf,
        &cancellation,
    );
    assert_eq!(result.err().unwrap(), "Preview cancelled");
    assert!(worker.join().unwrap());
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[test]
fn cancelled_office_requests_do_not_probe_or_start_a_converter() {
    let cancellation = Cancellation::default();
    cancellation.cancel();
    assert_eq!(
        render(Path::new("/unused/generated.docx"), 0, &cancellation)
            .err()
            .unwrap(),
        "Preview cancelled"
    );
}

fn sandboxed_conversion(input: &Path, cancellation: &Cancellation) -> Result<Vec<u8>, String> {
    sandboxed_conversion_with_font_cache(input, cancellation, true)
}

fn sandboxed_conversion_with_font_cache(
    input: &Path,
    cancellation: &Cancellation,
    system_font_cache: bool,
) -> Result<Vec<u8>, String> {
    let directory = PrivateOutput::create().unwrap();
    let helper = super::super::helper_executable().unwrap();
    let command = super::super::sandbox_command(
        &helper,
        input,
        directory.path(),
        ParseOperation::ConvertOfficePdf,
        0,
    )
    .unwrap();
    let mut arguments: Vec<_> = command.get_args().map(ToOwned::to_owned).collect();
    if !system_font_cache {
        let index = arguments
            .windows(3)
            .position(|values| {
                values[0] == "--ro-bind-try"
                    && values[1] == "/var/cache/fontconfig"
                    && values[2] == "/var/cache/fontconfig"
            })
            .unwrap();
        arguments.drain(index..index + 3);
    }
    let separator = arguments
        .iter()
        .position(|argument| argument == "--")
        .unwrap();
    let mut isolated = std::process::Command::new(command.get_program());
    isolated.args(&arguments[..separator]);
    if let Some(runtime) = std::env::var_os("HERMES_OFFICE_TEST_RUNTIME") {
        let runtime = PathBuf::from(runtime).canonicalize().unwrap();
        assert!(runtime.join("program/soffice").is_file());
        isolated
            .args(["--dir", "/opt", "--ro-bind"])
            .arg(runtime)
            .arg("/opt/libreoffice");
    }
    isolated.args(&arguments[separator..]);
    super::super::run_one_shot(
        isolated,
        &directory,
        ParseOperation::ConvertOfficePdf,
        cancellation,
    )
    .map(|output| output.data)
}

#[test]
#[ignore = "Explicit read-only Office sample, built helper and system LibreOffice; paired uncached exports"]
fn uncached_office_font_cache_comparison_retains_layout_and_original() {
    let input = PathBuf::from(
        std::env::var_os("HERMES_OFFICE_TEST_FILE").expect("Choose a read-only Office sample"),
    );
    let original = fs::read(&input).unwrap();
    let cancellation = Cancellation::default();
    let mut reference = None;
    for system_font_cache in [false, true, true, false] {
        let started = Instant::now();
        let bytes =
            sandboxed_conversion_with_font_cache(&input, &cancellation, system_font_cache).unwrap();
        let elapsed = started.elapsed();
        let directory = PrivateOutput::create().unwrap();
        let pdf = directory.path().join("converted.pdf");
        fs::write(&pdf, &bytes).unwrap();
        let page = super::super::parse(&pdf, ParseOperation::PreviewPdf, 0, &cancellation).unwrap();
        let layout = (page.pages, page.text_layer);
        assert!(layout.0 > 0);
        assert!(layout.1.is_some());
        if let Some(expected) = reference.as_ref() {
            assert_eq!(&layout, expected);
        } else {
            reference = Some(layout);
        }
        assert_eq!(fs::read(&input).unwrap(), original);
        eprintln!(
            "Uncached Office export: system_font_cache={system_font_cache}, elapsed={elapsed:?}, bytes={}",
            bytes.len()
        );
    }
}

#[test]
#[ignore = "Explicit read-only local Office sample; requires a system LibreOffice renderer"]
fn local_office_document_renders_first_page_without_text_fallback() {
    let input = PathBuf::from(
        std::env::var_os("HERMES_OFFICE_TEST_FILE").expect("Choose a read-only Office sample"),
    );
    let original = fs::read(&input).unwrap();
    let started = Instant::now();
    let output = render(&input, 0, &Cancellation::default()).unwrap();
    assert!(output.pages > 0);
    assert!(output.data.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert_eq!(original, fs::read(&input).unwrap());
    eprintln!("Rendered {} pages in {:?}", output.pages, started.elapsed());
}

#[test]
fn office_admission_serializes_jobs_and_queued_cancellation_stops_before_starting() {
    let (started, admitted) = std::sync::mpsc::channel();
    let (release, released) = std::sync::mpsc::channel();
    let first = thread::spawn(move || {
        admit(&Cancellation::default(), false, || {
            started.send(()).unwrap();
            released.recv_timeout(Duration::from_secs(3)).unwrap();
            Ok(())
        })
    });
    admitted.recv_timeout(Duration::from_secs(3)).unwrap();
    let cancellation = Cancellation::default();
    let queued = cancellation.clone();
    let second = thread::spawn(move || {
        admit(&queued, false, || -> Result<(), String> {
            panic!("Canceled job started")
        })
    });
    cancellation.cancel();
    assert_eq!(second.join().unwrap().unwrap_err(), "Preview cancelled");
    release.send(()).unwrap();
    first.join().unwrap().unwrap();
}

#[test]
fn selected_office_conversion_precedes_waiting_thumbnail_exports() {
    let admission = Arc::new(Admission::default());
    let occupied = admission.converter.lock().unwrap();
    let (events, received) = std::sync::mpsc::channel();
    let background_admission = admission.clone();
    let background_events = events.clone();
    let background = thread::spawn(move || {
        background_admission.run(true, &Cancellation::default(), || {
            background_events.send("thumbnail").unwrap();
            Ok(())
        })
    });
    let selected_admission = admission.clone();
    let selected = thread::spawn(move || {
        selected_admission.run(false, &Cancellation::default(), || {
            events.send("selected").unwrap();
            Ok(())
        })
    });
    let started = Instant::now();
    while admission.foreground.load(Ordering::Acquire) == 0 {
        assert!(started.elapsed() < Duration::from_secs(3));
        thread::sleep(Duration::from_millis(1));
    }
    drop(occupied);
    assert_eq!(
        received.recv_timeout(Duration::from_secs(3)).unwrap(),
        "selected"
    );
    assert_eq!(
        received.recv_timeout(Duration::from_secs(3)).unwrap(),
        "thumbnail"
    );
    selected.join().unwrap().unwrap();
    background.join().unwrap().unwrap();
    assert_eq!(admission.foreground.load(Ordering::Acquire), 0);
}

#[test]
fn cancelled_selected_office_request_releases_thumbnail_priority() {
    let admission = Admission::default();
    let cancellation = Cancellation::default();
    cancellation.cancel();
    assert_eq!(
        admission.run(false, &cancellation, || Ok(())).unwrap_err(),
        "Preview cancelled"
    );
    assert_eq!(admission.foreground.load(Ordering::Acquire), 0);
    assert!(
        admission
            .run(true, &Cancellation::default(), || Ok(()))
            .is_ok()
    );
}

#[test]
#[ignore = "Generated Office fixtures and a system renderer; checks real icon thumbnail output"]
fn generated_office_thumbnails_render_documents_slides_and_workbooks() {
    let fixtures = PathBuf::from(std::env::var_os("HERMES_OFFICE_FIXTURE_DIR").unwrap());
    for name in ["layout.docx", "layout.odt", "layout.pptx", "layout.xlsx"] {
        let input = fixtures.join(name);
        let original = fs::read(&input).unwrap();
        let output = super::super::parse(
            &input,
            ParseOperation::ThumbnailOffice,
            96,
            &Cancellation::default(),
        )
        .unwrap();
        let dimensions = crate::sandbox_helper::sniff::sniff_png_dimensions(&output.data).unwrap();
        assert!(dimensions.width > 0 && dimensions.height > 0);
        assert!(dimensions.width.max(dimensions.height) <= 96);
        if name == "layout.xlsx" {
            assert!(dimensions.width > dimensions.height * 2);
        }
        assert_eq!(original, fs::read(input).unwrap());
        if let Some(shots) = std::env::var_os("HERMES_OFFICE_TEST_SHOTS") {
            fs::create_dir_all(&shots).unwrap();
            fs::write(
                Path::new(&shots).join(format!("{name}-icon.png")),
                output.data,
            )
            .unwrap();
        }
    }
}

#[test]
#[ignore = "Requires generated DOCX/PPTX fixtures, built helper and optional LibreOffice; no user documents"]
fn generated_office_pages_preserve_layout_selectable_text_and_cached_conversion() {
    let fixtures = PathBuf::from(
        std::env::var_os("HERMES_OFFICE_FIXTURE_DIR")
            .expect("Run tests/fixtures/office/generate.py first"),
    );
    for (name, first_text, second_text, landscape) in [
        ("layout.docx", "Hermes layout fixture", "marmalade", false),
        ("layout.odt", "Hermes ODT fixture", "ODT marmalade", false),
        ("layout.pptx", "Hermes slide 1", "Hermes slide 2", true),
    ] {
        let input = fixtures.join(name);
        let original = fs::read(&input).unwrap();
        let cache = Mutex::new(Cache::default());
        let cancellation = Cancellation::default();
        let first = render_cached(&input, 0, &cancellation, &cache, |source| {
            sandboxed_conversion(source, &cancellation)
        })
        .unwrap();
        assert_eq!((first.page, first.pages), (0, 2));
        let first_layer = first.text_layer.as_ref().unwrap();
        assert!(
            first_layer.text.contains(first_text),
            "{}",
            first_layer.text
        );
        assert!(!first_layer.text.contains(second_text));
        assert!(!first_layer.glyphs.is_empty());
        assert_eq!(first_layer.width > first_layer.height, landscape);
        let second = render_cached(&input, 1, &cancellation, &cache, |_| {
            panic!("Page changes must reuse conversion")
        })
        .unwrap();
        assert_eq!((second.page, second.pages), (1, 2));
        assert!(
            second
                .text_layer
                .as_ref()
                .unwrap()
                .text
                .contains(second_text)
        );
        assert_ne!(first.data, second.data);
        let last = render_cached(&input, 999, &cancellation, &cache, |_| {
            panic!("Still cached")
        })
        .unwrap();
        assert_eq!((last.page, last.pages), (1, 2));
        assert_eq!(original, fs::read(&input).unwrap());
        if let Some(shots) = std::env::var_os("HERMES_OFFICE_TEST_SHOTS") {
            fs::create_dir_all(&shots).unwrap();
            fs::write(Path::new(&shots).join(format!("{name}-1.png")), first.data).unwrap();
            fs::write(Path::new(&shots).join(format!("{name}-2.png")), second.data).unwrap();
        }
    }
}
