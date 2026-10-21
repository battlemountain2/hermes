use super::*;

#[derive(Clone, Copy, Default, Debug)]
struct Statistics {
    lookups: usize,
    renders: usize,
    decoded: usize,
    peak_running: usize,
}

thread_local! {
    static STATISTICS: RefCell<Statistics> = RefCell::new(Statistics::default());
}

pub(super) fn started(render: bool) {
    STATISTICS.with(|statistics| {
        let mut statistics = statistics.borrow_mut();
        if render {
            statistics.renders += 1;
        } else {
            statistics.lookups += 1;
        }
        statistics.peak_running = statistics
            .peak_running
            .max(QUEUE.with(|queue| queue.borrow().running.len()));
    });
}

pub(super) fn rendered(decoded: bool) {
    STATISTICS.with(|statistics| statistics.borrow_mut().decoded += usize::from(decoded));
}

#[derive(Clone)]
struct Subscriber {
    alive: bool,
    priority: Option<u8>,
}

fn subscriber(priority: Option<u8>) -> Subscriber {
    Subscriber {
        alive: true,
        priority,
    }
}

fn start(queue: &mut Queue<usize, Subscriber>, limit: usize) -> Option<(u64, usize, Cancellation)> {
    queue.start(limit, |target| target.priority, |_| true)
}

#[test]
fn visible_work_overtakes_prefetch_and_never_starts_hidden_work() {
    let mut queue = Queue::default();
    queue.enqueue(1, subscriber(None), false);
    queue.enqueue(2, subscriber(Some(1)), false);
    queue.enqueue(3, subscriber(Some(0)), false);
    assert_eq!(start(&mut queue, 4).unwrap().1, 3);
    assert_eq!(start(&mut queue, 4).unwrap().1, 2);
    assert!(start(&mut queue, 4).is_none());
    queue.waiting[0].targets[0].priority = Some(0);
    assert_eq!(start(&mut queue, 4).unwrap().1, 1);
}

#[test]
fn worker_admission_stays_bounded_until_cancelled_workers_really_finish() {
    let mut queue = Queue::default();
    for work in 0..1000 {
        queue.enqueue(work, subscriber(Some(0)), false);
    }
    let first = start(&mut queue, 4).unwrap();
    for _ in 0..3 {
        assert!(start(&mut queue, 4).is_some());
    }
    assert!(start(&mut queue, 4).is_none());
    queue.running[0].job.targets[0].priority = None;
    queue.maintain(|target| target.alive, |target| target.priority);
    assert!(first.2.is_cancelled());
    assert!(start(&mut queue, 4).is_none());
    queue.finish(first.0).unwrap();
    assert!(start(&mut queue, 4).is_some());
    assert_eq!(queue.running.len(), 4);
}

#[test]
fn duplicate_waiters_share_work_and_one_cancellation_does_not_cancel_the_other() {
    let mut queue = Queue::default();
    queue.enqueue(7, subscriber(Some(0)), true);
    let job = start(&mut queue, 4).unwrap();
    queue.enqueue(7, subscriber(Some(0)), true);
    assert!(queue.waiting.is_empty());
    assert_eq!(queue.running.len(), 1);
    queue.running[0].job.targets[0].alive = false;
    queue.maintain(|target| target.alive, |target| target.priority);
    assert!(!job.2.is_cancelled());
    let completed = queue.finish(job.0).unwrap();
    assert_eq!(completed.job.targets.len(), 1);
    assert!(queue.finish(job.0).is_none());
}

#[test]
fn a_cancelled_flight_is_never_joined_and_dead_pending_requests_are_removed() {
    let mut queue = Queue::default();
    queue.enqueue(7, subscriber(Some(0)), true);
    let job = start(&mut queue, 4).unwrap();
    job.2.cancel();
    queue.enqueue(7, subscriber(Some(0)), true);
    assert_eq!(queue.waiting.len(), 1);
    queue.waiting[0].targets[0].alive = false;
    queue.maintain(|target| target.alive, |target| target.priority);
    assert!(queue.waiting.is_empty());
}

#[test]
fn unresolved_running_work_is_not_shared_without_fresh_identity_checks() {
    let mut queue = Queue::default();
    queue.enqueue(7, subscriber(Some(0)), false);
    let _first = start(&mut queue, 4).unwrap();
    queue.enqueue(7, subscriber(Some(0)), false);
    assert_eq!(queue.waiting.len(), 1);
    assert_eq!(queue.running[0].job.targets.len(), 1);
}

#[test]
fn changed_source_or_artwork_identity_and_size_never_share_a_render() {
    let identity = SourceIdentity {
        device: 1,
        inode: 2,
        size: 3,
        modified: (4, 5),
        changed: (6, 7),
    };
    let source = ThumbnailSource {
        key: ThumbnailKey {
            path: PathBuf::from("/generated/track.flac"),
            identity,
            source_path: PathBuf::from("/generated/cover.jpg"),
            source_identity: identity,
            thumbnail_size: 64,
        },
        operation: ParseOperation::ThumbnailImage,
    };
    let mut queue = Queue::default();
    let original = Work::Render(source.clone(), ThumbnailKind::AudioAlbum);
    queue.enqueue(original.clone(), subscriber(Some(0)), true);
    let _started = queue
        .start(4, |target| target.priority, Work::render)
        .unwrap();
    queue.enqueue(original, subscriber(Some(0)), true);
    for variant in 0..4 {
        let mut changed = source.clone();
        match variant {
            0 => changed.key.identity.changed.1 += 1,
            1 => changed.key.source_identity.changed.1 += 1,
            2 => changed.key.thumbnail_size = 128,
            _ => changed.operation = ParseOperation::ThumbnailVideo,
        }
        queue.enqueue(
            Work::Render(changed, ThumbnailKind::AudioAlbum),
            subscriber(Some(0)),
            true,
        );
    }
    assert_eq!(queue.running[0].job.targets.len(), 2);
    assert_eq!(queue.waiting.len(), 4);
}

#[test]
fn already_resolved_visible_work_finishes_before_another_visible_metadata_lookup() {
    let mut queue = Queue::default();
    queue.enqueue(1, subscriber(Some(0)), false);
    queue.enqueue(2, subscriber(Some(0)), false);
    assert_eq!(
        queue
            .start(4, |target| target.priority, |work| *work == 2)
            .unwrap()
            .1,
        2
    );
}

#[test]
#[ignore = "Private GTK display/cache and built helper; generated shared render only"]
fn desktop_rebound_shared_thumbnail_does_not_cancel_or_overwrite_its_peer() {
    let root = PathBuf::from(std::env::var_os("HERMES_PREVIEW_PROBE").unwrap());
    assert!(root.starts_with("/tmp"));
    assert!(PathBuf::from(std::env::var_os("XDG_CACHE_HOME").unwrap()).starts_with(&root));
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let fixture = crate::sandbox::browser::format_tests::FixtureDirectory::new();
    let pixels =
        gtk::gdk_pixbuf::Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, true, 8, 256, 128).unwrap();
    pixels.fill(0x2060c0ff);
    let path = fixture.0.join("shared.png");
    std::fs::write(&path, pixels.save_to_bufferv("png", &[]).unwrap()).unwrap();
    let first = gtk::Image::new();
    let peer = gtk::Image::new();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&first);
    content.append(&peer);
    let scroll = gtk::ScrolledWindow::builder().child(&content).build();
    let window = gtk::Window::builder()
        .child(&scroll)
        .default_width(400)
        .default_height(300)
        .build();
    let mut targets = Vec::new();
    for image in [&first, &peer] {
        let (image_id, request, cancellation) =
            super::super::set_fallback_icon(image, crate::assets::icons::DOCUMENTS, 32);
        targets.push(Target {
            image: image.downgrade(),
            image_id,
            request,
            cancellation,
        });
    }
    let pump = || {
        for _ in 0..64 {
            if !glib::MainContext::default().iteration(false) {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    };
    let until = |condition: &dyn Fn() -> bool| {
        let deadline = std::time::Instant::now() + Duration::from_secs(12);
        while !condition() {
            assert!(std::time::Instant::now() < deadline);
            pump();
        }
    };
    window.present();
    until(&|| visible_priority(&first).is_some() && visible_priority(&peer).is_some());
    let source = thumbnail_source(
        &path,
        ThumbnailKind::Sandboxed(ParseOperation::ThumbnailImage),
        64,
        None,
        folder_album_art,
    )
    .unwrap();
    QUEUE.with(|queue| {
        let mut queue = queue.borrow_mut();
        for target in targets {
            queue.enqueue(
                Work::Render(
                    source.clone(),
                    ThumbnailKind::Sandboxed(ParseOperation::ThumbnailImage),
                ),
                target,
                true,
            );
        }
    });
    tick();
    assert_eq!(QUEUE.with(|queue| queue.borrow().running.len()), 1);
    super::super::show_fallback_icon(&first, crate::assets::icons::FOLDER, 32);
    until(&|| !crate::assets::has_primary_icon(&peer));
    until(&|| QUEUE.with(|queue| queue.borrow().running.is_empty()));
    assert!(crate::assets::has_primary_icon(&first));
    assert_eq!(STATISTICS.with(|statistics| statistics.borrow().decoded), 1);
    let popover = gtk::Popover::new();
    popover.set_parent(&first);
    popover.set_position(gtk::PositionType::Right);
    let popup_image = gtk::Image::new();
    popup_image.set_size_request(64, 64);
    popover.set_child(Some(&popup_image));
    popover.popup();
    until(&|| popup_image.is_mapped() && popup_image.width() > 0);
    assert_eq!(visible_priority(&popup_image), Some(0));
    popover.popdown();
    popover.unparent();
    window.close();
    super::super::cancel_thumbnail(&first);
    super::super::cancel_thumbnail(&peer);
    until(&|| QUEUE.with(|queue| queue.borrow().waiting.is_empty()));
}

#[test]
#[ignore = "Private GTK display/cache and built helper; generated images only"]
fn desktop_visible_thumbnails_share_decoding_and_hidden_tabs_wait_until_shown() {
    let root = PathBuf::from(std::env::var_os("HERMES_PREVIEW_PROBE").unwrap());
    assert!(root.starts_with("/tmp"));
    assert!(PathBuf::from(std::env::var_os("XDG_CACHE_HOME").unwrap()).starts_with(&root));
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let fixture = crate::sandbox::browser::format_tests::FixtureDirectory::new();
    let pixels =
        gtk::gdk_pixbuf::Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, true, 8, 256, 128).unwrap();
    pixels.fill(0x2060c0ff);
    let png = pixels.save_to_bufferv("png", &[]).unwrap();
    let entry = |name: &str| {
        let path = fixture.0.join(name);
        std::fs::write(&path, &png).unwrap();
        crate::model::FileEntry(std::rc::Rc::new(crate::model::FileEntryInner {
            location: crate::model::Location::local(&path),
            native_name: path.file_name().unwrap().to_owned(),
            display_name: name.into(),
            kind: crate::model::EntryKind::File,
            size: crate::model::MetadataValue::Unknown,
            modified_unix_seconds: crate::model::MetadataValue::Unknown,
            music_tags: None,
        }))
    };
    let same = entry("same.png");
    let below = entry("below.png");
    let hidden = entry("hidden.png");
    let first = gtk::Image::new();
    let duplicate = gtk::Image::new();
    let lower = gtk::Image::new();
    let other_tab = gtk::Image::new();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.append(&first);
    content.append(&duplicate);
    let spacer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    spacer.set_height_request(1200);
    content.append(&spacer);
    content.append(&lower);
    let scroll = gtk::ScrolledWindow::builder().child(&content).build();
    let tabs = gtk::Stack::new();
    tabs.add_named(&scroll, Some("files"));
    tabs.add_named(&other_tab, Some("other"));
    tabs.set_visible_child_name("files");
    let window = gtk::Window::builder()
        .child(&tabs)
        .default_width(400)
        .default_height(300)
        .build();
    for (image, entry) in [
        (&first, &same),
        (&duplicate, &same),
        (&lower, &below),
        (&other_tab, &hidden),
    ] {
        super::super::set_thumbnail_or_icon(image, entry, crate::assets::icons::DOCUMENTS, 32, 64);
    }
    let pump = || {
        for _ in 0..64 {
            if !glib::MainContext::default().iteration(false) {
                break;
            }
        }
        if let Some(clock) = window.frame_clock() {
            clock.emit_by_name::<()>("update", &[]);
            clock.emit_by_name::<()>("layout", &[]);
        }
        std::thread::sleep(Duration::from_millis(2));
    };
    let has_thumbnail =
        |image: &gtk::Image| !crate::assets::has_primary_icon(image) && image.paintable().is_some();
    let until = |condition: &dyn Fn() -> bool| {
        let deadline = std::time::Instant::now() + Duration::from_secs(12);
        while !condition() {
            assert!(
                std::time::Instant::now() < deadline,
                "{:?}",
                STATISTICS.with(|statistics| *statistics.borrow())
            );
            pump();
        }
    };
    let started = std::time::Instant::now();
    window.present();
    until(&|| has_thumbnail(&first) && has_thumbnail(&duplicate));
    let first_time = started.elapsed();
    until(&|| QUEUE.with(|queue| queue.borrow().running.is_empty()));
    assert!(!has_thumbnail(&lower));
    assert!(!has_thumbnail(&other_tab));
    assert_eq!(STATISTICS.with(|statistics| statistics.borrow().decoded), 1);
    assert_eq!(STATISTICS.with(|statistics| statistics.borrow().renders), 1);
    assert_eq!(visible_priority(&lower), None);
    assert_eq!(visible_priority(&other_tab), None);
    let started = std::time::Instant::now();
    super::super::set_thumbnail_or_icon(&first, &same, crate::assets::icons::DOCUMENTS, 32, 64);
    until(&|| has_thumbnail(&first));
    let cached_time = started.elapsed();
    assert_eq!(STATISTICS.with(|statistics| statistics.borrow().decoded), 1);
    let adjustment = scroll.vadjustment();
    adjustment.set_value(adjustment.upper() - adjustment.page_size());
    until(&|| has_thumbnail(&lower));
    tabs.set_visible_child_name("other");
    until(&|| has_thumbnail(&other_tab));
    until(&|| QUEUE.with(|queue| queue.borrow().running.is_empty()));
    let statistics = STATISTICS.with(|statistics| *statistics.borrow());
    assert_eq!(statistics.decoded, 3);
    assert!(statistics.peak_running <= crate::sandbox::browser::default_worker_limit());
    eprintln!(
        "Visible duplicate pair ready in {first_time:?}; warm rebind in {cached_time:?}; {statistics:?}"
    );
    window.close();
    for image in [&first, &duplicate, &lower, &other_tab] {
        super::super::cancel_thumbnail(image);
    }
    until(&|| {
        QUEUE.with(|queue| queue.borrow().waiting.is_empty() && queue.borrow().running.is_empty())
    });
}
