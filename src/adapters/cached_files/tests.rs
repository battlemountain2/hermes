use super::*;
use crate::model::{EntryKind, FileEntryInner, MetadataValue, MusicTags};
use std::{ffi::OsString, os::unix::ffi::OsStringExt};

#[derive(Default)]
struct FakeSource {
    calls: RefCell<Vec<(DirectoryRequest, Rc<dyn Fn(DirectoryEvent)>, Rc<Cell<bool>>)>>,
    watches: RefCell<Vec<(Location, Rc<dyn Fn()>, Rc<Cell<bool>>)>>,
    order: RefCell<Vec<&'static str>>,
    unsupported: Cell<bool>,
    invalidated: RefCell<Vec<Location>>,
    ui_watch: Cell<usize>,
}

impl FakeSource {
    fn emit(&self, index: usize, event: DirectoryEvent) {
        let callback = self.calls.borrow()[index].1.clone();
        callback(event);
    }

    fn complete(&self, index: usize, entries: Vec<FileEntry>) {
        let id = self.calls.borrow()[index].0.id;
        self.emit(
            index,
            DirectoryEvent::Batch {
                request_id: id,
                entries,
            },
        );
        self.emit(index, DirectoryEvent::Finished { request_id: id });
    }

    fn change(&self, location: &Location) {
        let callbacks = self
            .watches
            .borrow()
            .iter()
            .filter(|(watched, _, active)| watched == location && active.get())
            .map(|(_, callback, _)| callback.clone())
            .collect::<Vec<_>>();
        for callback in callbacks {
            callback();
        }
    }

    fn live_watches(&self) -> usize {
        self.watches
            .borrow()
            .iter()
            .filter(|(_, _, active)| active.get())
            .count()
    }
}

impl FileSource for FakeSource {
    fn validate_location(&self, _: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        self.order.borrow_mut().push("enumerate");
        let active = Rc::new(Cell::new(true));
        self.calls
            .borrow_mut()
            .push((request, emit, active.clone()));
        LoadHandle::new(move || active.set(false))
    }

    fn invalidate(&self, location: &Location) {
        self.invalidated.borrow_mut().push(location.clone());
    }

    fn watch_invalidation(&self, location: &Location, notify: Rc<dyn Fn()>) -> Option<LoadHandle> {
        self.order.borrow_mut().push("watch");
        if self.unsupported.get() {
            return None;
        }
        let active = Rc::new(Cell::new(true));
        self.watches
            .borrow_mut()
            .push((location.clone(), notify, active.clone()));
        Some(LoadHandle::new(move || active.set(false)))
    }

    fn watch(&self, _: Location, _: bool, _: Rc<dyn Fn(DirectoryChange)>) -> Option<LoadHandle> {
        self.ui_watch.set(self.ui_watch.get() + 1);
        Some(LoadHandle::new(|| {}))
    }
}

fn entry(name: &str) -> FileEntry {
    FileEntry(Rc::new(FileEntryInner {
        location: Location::local(format!("/fixture/{name}")),
        native_name: name.into(),
        display_name: name.into(),
        kind: EntryKind::File,
        size: MetadataValue::Known(5),
        modified_unix_seconds: MetadataValue::Known(10),
        music_tags: None,
    }))
}

fn request(id: u64, folder: &str) -> DirectoryRequest {
    DirectoryRequest {
        id: RequestId(id),
        location: Location::local(folder),
        batch_size: 128,
        include_hidden: false,
    }
}

fn sink() -> (Rc<RefCell<Vec<DirectoryEvent>>>, Rc<dyn Fn(DirectoryEvent)>) {
    let events = Rc::new(RefCell::new(Vec::new()));
    let received = events.clone();
    (
        events,
        Rc::new(move |event| received.borrow_mut().push(event)),
    )
}

fn source(fake: &Rc<FakeSource>) -> CachedFileSource {
    CachedFileSource::with_limits(fake.clone(), Limits::default(), Rc::new(Instant::now))
}

fn on_context(test: impl FnOnce(&glib::MainContext)) {
    let context = glib::MainContext::new();
    context.with_thread_default(|| test(&context)).unwrap();
    while context.iteration(false) {}
}

fn until(context: &glib::MainContext, done: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !done() {
        assert!(Instant::now() < deadline, "main context timed out");
        context.iteration(false);
    }
}

fn finished(events: &RefCell<Vec<DirectoryEvent>>) -> bool {
    events
        .borrow()
        .iter()
        .any(|event| matches!(event, DirectoryEvent::Finished { .. }))
}

#[test]
fn hit_is_async_batched_remaps_ids_and_preserves_native_entries() {
    on_context(|context| {
        let fake = Rc::new(FakeSource::default());
        let source = source(&fake);
        let cold = source.enumerate(request(1, "/fixture"), sink().1);
        assert_eq!(&*fake.order.borrow(), &["watch", "enumerate"]);
        let mut native = entry("link");
        let inner = Rc::make_mut(&mut native.0);
        inner.native_name = OsString::from_vec(b"link-\xff".to_vec());
        inner.location =
            Location::local(std::path::PathBuf::from("/fixture").join(&inner.native_name));
        inner.kind = EntryKind::DirectorySymbolicLink;
        let mut expected = vec![native];
        expected.extend((0..300).map(|index| entry(&format!("entry-{index}"))));
        fake.complete(0, expected.clone());
        drop(cold);
        assert_eq!(fake.live_watches(), 1);
        let (events, emit) = sink();
        let hit = source.enumerate(request(99, "/fixture"), emit);
        assert!(events.borrow().is_empty());
        until(context, || finished(&events));
        let batches = events
            .borrow()
            .iter()
            .filter_map(|event| match event {
                DirectoryEvent::Batch {
                    request_id,
                    entries,
                } => {
                    assert_eq!(*request_id, RequestId(99));
                    Some(entries.clone())
                }
                DirectoryEvent::Finished { request_id } => {
                    assert_eq!(*request_id, RequestId(99));
                    None
                }
                _ => panic!("unexpected replay event"),
            })
            .collect::<Vec<_>>();
        assert_eq!(batches[0].len(), 32);
        assert!(batches.iter().all(|batch| batch.len() <= 128));
        assert_eq!(batches.concat(), expected);
        assert_eq!(fake.calls.borrow().len(), 1);
        drop(hit);
    });
}

#[test]
fn hidden_preferences_have_independent_snapshots() {
    on_context(|context| {
        let fake = Rc::new(FakeSource::default());
        let source = source(&fake);
        let visible = source.enumerate(request(1, "/fixture"), sink().1);
        fake.complete(0, vec![entry("visible")]);
        let mut hidden_request = request(2, "/fixture");
        hidden_request.include_hidden = true;
        let hidden = source.enumerate(hidden_request, sink().1);
        fake.complete(1, vec![entry("visible"), entry(".hidden")]);
        drop((visible, hidden));
        let (events, emit) = sink();
        let hit = source.enumerate(request(3, "/fixture"), emit);
        until(context, || finished(&events));
        assert!(
            matches!(&events.borrow()[0], DirectoryEvent::Batch { entries, .. } if entries.len() == 1)
        );
        assert_eq!(fake.calls.borrow().len(), 2);
        fake.change(&Location::local("/fixture"));
        let miss = source.enumerate(request(4, "/fixture"), sink().1);
        assert_eq!(fake.calls.borrow().len(), 3);
        assert_eq!(fake.live_watches(), 1);
        drop((hit, miss));
    });
}

#[test]
fn inactive_watch_invalidates_and_explicit_refresh_delegates() {
    let fake = Rc::new(FakeSource::default());
    let source = source(&fake);
    let cold = source.enumerate(request(1, "/fixture"), sink().1);
    fake.complete(0, vec![entry("one")]);
    drop(cold);
    fake.change(&Location::local("/fixture"));
    let next = source.enumerate(request(2, "/fixture"), sink().1);
    assert_eq!(fake.calls.borrow().len(), 2);
    fake.complete(1, vec![entry("two")]);
    source.invalidate(&Location::local("/fixture"));
    assert_eq!(fake.live_watches(), 0);
    assert_eq!(&*fake.invalidated.borrow(), &[Location::local("/fixture")]);
    let fresh = source.enumerate(request(3, "/fixture"), sink().1);
    assert_eq!(fake.calls.borrow().len(), 3);
    drop((next, fresh));
}

#[test]
fn changes_during_load_never_promote() {
    let fake = Rc::new(FakeSource::default());
    let source = source(&fake);
    let first = source.enumerate(request(1, "/fixture"), sink().1);
    fake.change(&Location::local("/fixture"));
    fake.complete(0, vec![entry("old")]);
    let next = source.enumerate(request(2, "/fixture"), sink().1);
    assert_eq!(fake.calls.borrow().len(), 2);
    drop((first, next));
}

#[test]
fn provider_restart_never_promotes_partial_listing() {
    let fake = Rc::new(FakeSource::default());
    let source = source(&fake);
    let first = source.enumerate(request(1, "/fixture"), sink().1);
    fake.emit(
        0,
        DirectoryEvent::Batch {
            request_id: RequestId(1),
            entries: vec![entry("partial")],
        },
    );
    fake.emit(
        0,
        DirectoryEvent::Restarted {
            request_id: RequestId(1),
        },
    );
    fake.complete(0, vec![entry("fresh")]);
    assert!(source.cache.borrow().listings.is_empty());
    let next = source.enumerate(request(2, "/fixture"), sink().1);
    assert_eq!(fake.calls.borrow().len(), 2);
    drop((first, next));
}

#[test]
fn unmonitored_and_non_native_sources_never_reuse() {
    for location in [
        Location::local("/fixture"),
        Location::uri("sftp://host/folder"),
        Location::uri("search://query"),
        Location::local("relative"),
    ] {
        let fake = Rc::new(FakeSource::default());
        fake.unsupported.set(true);
        let source = source(&fake);
        let mut load_request = request(1, "/fixture");
        load_request.location = location;
        let first = source.enumerate(load_request.clone(), sink().1);
        fake.complete(0, vec![entry("one")]);
        load_request.id = RequestId(2);
        let second = source.enumerate(load_request, sink().1);
        assert_eq!(fake.calls.borrow().len(), 2);
        assert!(source.cache.borrow().listings.is_empty());
        drop((first, second));
    }
}

#[test]
fn cancellation_failure_and_wrong_request_ids_never_promote() {
    for scenario in 0..4 {
        let fake = Rc::new(FakeSource::default());
        let source = source(&fake);
        let (events, emit) = sink();
        let load = source.enumerate(request(1, "/fixture"), emit);
        fake.emit(
            0,
            DirectoryEvent::Batch {
                request_id: RequestId(1),
                entries: vec![entry("one")],
            },
        );
        if scenario == 0 {
            drop(load);
            fake.complete(0, vec![entry("late")]);
            assert_eq!(events.borrow().len(), 1);
        } else {
            if scenario == 1 {
                fake.emit(
                    0,
                    DirectoryEvent::Failed {
                        request_id: RequestId(1),
                        message: "interrupted".into(),
                    },
                );
            } else if scenario == 2 {
                fake.emit(
                    0,
                    DirectoryEvent::Finished {
                        request_id: RequestId(999),
                    },
                );
            }
            drop(load);
        }
        let next = source.enumerate(request(2, "/fixture"), sink().1);
        assert_eq!(fake.calls.borrow().len(), 2);
        drop(next);
    }
}

#[test]
fn stale_replaced_load_cannot_overwrite_new_listing() {
    on_context(|context| {
        let fake = Rc::new(FakeSource::default());
        let source = source(&fake);
        let old = source.enumerate(request(1, "/fixture"), sink().1);
        let new = source.enumerate(request(2, "/fixture"), sink().1);
        fake.complete(1, vec![entry("new")]);
        fake.complete(0, vec![entry("old")]);
        drop((old, new));
        let (events, emit) = sink();
        let hit = source.enumerate(request(3, "/fixture"), emit);
        until(context, || finished(&events));
        assert!(
            matches!(&events.borrow()[0], DirectoryEvent::Batch { entries, .. } if entries == &vec![entry("new")])
        );
        drop(hit);
    });
}

#[test]
fn metadata_after_finished_is_accounted_and_replayed() {
    on_context(|context| {
        let fake = Rc::new(FakeSource::default());
        let source = source(&fake);
        let cold = source.enumerate(request(1, "/fixture"), sink().1);
        let song = entry("song.ogg");
        fake.complete(0, vec![song.clone()]);
        let previous_bytes = source.cache.borrow().listings[0].bytes;
        let mut tagged = song;
        Rc::make_mut(&mut tagged.0).music_tags = Some(MusicTags {
            title: Some("Hydrated title".into()),
            album_artist: Some("Artist".into()),
            ..MusicTags::default()
        });
        fake.emit(
            0,
            DirectoryEvent::MusicMetadata {
                request_id: RequestId(1),
                entries: vec![tagged.clone(), entry("not-in-listing")],
            },
        );
        assert!(source.cache.borrow().listings[0].bytes > previous_bytes);
        drop(cold);
        let (events, emit) = sink();
        let hit = source.enumerate(request(42, "/fixture"), emit);
        until(context, || finished(&events));
        assert!(
            matches!(&events.borrow()[0], DirectoryEvent::Batch { request_id, entries } if *request_id == RequestId(42) && entries == &vec![tagged])
        );
        drop(hit);
    });
}

#[test]
fn metadata_arriving_during_hit_is_replayed_after_finished_with_new_id() {
    on_context(|context| {
        let fake = Rc::new(FakeSource::default());
        let source = source(&fake);
        let cold = source.enumerate(request(1, "/fixture"), sink().1);
        fake.complete(0, vec![entry("song.bin")]);
        let mut tagged = entry("song.bin");
        Rc::make_mut(&mut tagged.0).music_tags = Some(MusicTags {
            title: Some("late title".into()),
            ..MusicTags::default()
        });
        let (events, record) = sink();
        let incoming = fake.clone();
        let expected = tagged.clone();
        let hit = source.enumerate(
            request(42, "/fixture"),
            Rc::new(move |event| {
                if matches!(event, DirectoryEvent::Batch { .. }) {
                    incoming.emit(
                        0,
                        DirectoryEvent::MusicMetadata {
                            request_id: RequestId(1),
                            entries: vec![tagged.clone()],
                        },
                    );
                }
                record(event);
            }),
        );
        until(context, || {
            events
                .borrow()
                .iter()
                .any(|event| matches!(event, DirectoryEvent::MusicMetadata { .. }))
        });
        let events = events.borrow();
        let finished_position = events.iter().position(|event| matches!(event, DirectoryEvent::Finished { request_id } if *request_id == RequestId(42))).unwrap();
        let metadata_position = events.iter().position(|event| matches!(event, DirectoryEvent::MusicMetadata { request_id, entries } if *request_id == RequestId(42) && entries == &vec![expected.clone()])).unwrap();
        assert!(metadata_position > finished_position);
        drop((cold, hit));
    });
}

#[test]
fn replay_cancel_before_first_batch_and_between_batches_is_safe() {
    on_context(|context| {
        let fake = Rc::new(FakeSource::default());
        let source = source(&fake);
        let cold = source.enumerate(request(1, "/fixture"), sink().1);
        fake.complete(
            0,
            (0..1000).map(|index| entry(&index.to_string())).collect(),
        );
        drop(cold);
        let (events, emit) = sink();
        drop(source.enumerate(request(2, "/fixture"), emit));
        while context.iteration(false) {}
        assert!(events.borrow().is_empty());
        let holder = Rc::new(RefCell::new(None::<LoadHandle>));
        let cancel = holder.clone();
        let batches = Rc::new(Cell::new(0));
        let received = batches.clone();
        holder.replace(Some(source.enumerate(
            request(3, "/fixture"),
            Rc::new(move |event| {
                assert!(matches!(event, DirectoryEvent::Batch { .. }));
                received.set(received.get() + 1);
                cancel.take();
            }),
        )));
        until(context, || batches.get() > 0);
        while context.iteration(false) {}
        assert_eq!(batches.get(), 1);
        assert_eq!(fake.calls.borrow().len(), 1);
    });
}

#[test]
fn invalidated_replay_resets_and_retains_cancel_safe_fresh_load() {
    on_context(|context| {
        let fake = Rc::new(FakeSource::default());
        let source = Rc::new(source(&fake));
        let cold = source.enumerate(request(1, "/fixture"), sink().1);
        fake.complete(0, (0..200).map(|index| entry(&index.to_string())).collect());
        drop(cold);
        let (events, record) = sink();
        let changed = source.clone();
        let hit = source.enumerate(
            request(2, "/fixture"),
            Rc::new(move |event| {
                if matches!(event, DirectoryEvent::Batch { .. }) {
                    changed.invalidate(&Location::local("/fixture"));
                }
                record(event);
            }),
        );
        until(context, || fake.calls.borrow().len() == 2);
        assert!(
            matches!(&events.borrow()[0], DirectoryEvent::Batch { entries, .. } if entries.len() == 32)
        );
        assert!(
            matches!(&events.borrow()[1], DirectoryEvent::Restarted { request_id } if *request_id == RequestId(2))
        );
        assert!(fake.calls.borrow()[1].2.get());
        fake.complete(1, vec![entry("fresh")]);
        assert!(finished(&events));
        assert!(
            !events
                .borrow()
                .iter()
                .any(|event| matches!(event, DirectoryEvent::Failed { .. }))
        );
        let previous = events.borrow().len();
        drop(hit);
        assert!(!fake.calls.borrow()[1].2.get());
        fake.emit(
            1,
            DirectoryEvent::MusicMetadata {
                request_id: RequestId(2),
                entries: vec![entry("late")],
            },
        );
        assert_eq!(events.borrow().len(), previous);
    });
}

#[test]
fn invalidation_before_replay_fresh_failure_reports_actual_provider_reason() {
    on_context(|context| {
        let fake = Rc::new(FakeSource::default());
        let source = source(&fake);
        let cold = source.enumerate(request(1, "/fixture"), sink().1);
        fake.complete(0, vec![entry("one")]);
        drop(cold);
        let (events, emit) = sink();
        let hit = source.enumerate(request(2, "/fixture"), emit);
        source.invalidate(&Location::local("/fixture"));
        until(context, || fake.calls.borrow().len() == 2);
        fake.emit(
            1,
            DirectoryEvent::Failed {
                request_id: RequestId(2),
                message: "Permission denied".into(),
            },
        );
        assert!(
            matches!(&events.borrow()[1], DirectoryEvent::Failed { message, .. } if message == "Permission denied")
        );
        drop(hit);
    });
}

#[test]
fn directory_limit_and_lru_release_watches() {
    on_context(|context| {
        let fake = Rc::new(FakeSource::default());
        let source = source(&fake);
        for index in 0..12 {
            let load = source.enumerate(request(index, &format!("/dir-{index}")), sink().1);
            fake.complete(index as usize, vec![entry("one")]);
            drop(load);
        }
        let (events, emit) = sink();
        let hit = source.enumerate(request(20, "/dir-0"), emit);
        until(context, || finished(&events));
        let extra = source.enumerate(request(21, "/extra"), sink().1);
        fake.complete(12, vec![entry("extra")]);
        assert_eq!(source.cache.borrow().listings.len(), 12);
        assert_eq!(fake.live_watches(), 12);
        assert!(
            source
                .cache
                .borrow()
                .listings
                .iter()
                .any(|listing| listing.location == Location::local("/dir-0"))
        );
        assert!(
            !source
                .cache
                .borrow()
                .listings
                .iter()
                .any(|listing| listing.location == Location::local("/dir-1"))
        );
        drop((extra, hit));
        drop(source);
        assert_eq!(fake.live_watches(), 0);
    });
}

#[test]
fn byte_limit_bounds_partial_accumulator_and_includes_tags() {
    let fake = Rc::new(FakeSource::default());
    let one = entry("one");
    let budget = listing_bytes(&Location::local("/fixture")) + entry_bytes(&one) * 2;
    let source = CachedFileSource::with_limits(
        fake.clone(),
        Limits {
            bytes: budget,
            ..Limits::default()
        },
        Rc::new(Instant::now),
    );
    let load = source.enumerate(request(1, "/fixture"), sink().1);
    fake.emit(
        0,
        DirectoryEvent::Batch {
            request_id: RequestId(1),
            entries: vec![one.clone(), entry("two")],
        },
    );
    assert_eq!(source.cache.borrow().listings[0].entries.len(), 2);
    assert!(source.cache.borrow().listings[0].bytes <= budget);
    fake.emit(
        0,
        DirectoryEvent::Batch {
            request_id: RequestId(1),
            entries: vec![entry("three")],
        },
    );
    assert!(source.cache.borrow().listings.is_empty());
    fake.emit(
        0,
        DirectoryEvent::Finished {
            request_id: RequestId(1),
        },
    );
    assert!(source.cache.borrow().listings.is_empty());
    let next = source.enumerate(request(2, "/fixture"), sink().1);
    fake.complete(1, vec![one.clone()]);
    let mut tagged = one;
    Rc::make_mut(&mut tagged.0).music_tags = Some(MusicTags {
        album: Some("x".repeat(budget)),
        ..MusicTags::default()
    });
    fake.emit(
        1,
        DirectoryEvent::MusicMetadata {
            request_id: RequestId(2),
            entries: vec![tagged],
        },
    );
    assert!(source.cache.borrow().listings.is_empty());
    assert_eq!(fake.live_watches(), 0);
    drop((load, next));
}

#[test]
fn ttl_is_not_extended_by_hits_and_expiration_releases_watches() {
    on_context(|context| {
        let fake = Rc::new(FakeSource::default());
        let clock = Rc::new(Cell::new(Instant::now()));
        let time = clock.clone();
        let source = CachedFileSource::with_limits(
            fake.clone(),
            Limits::default(),
            Rc::new(move || time.get()),
        );
        let cold = source.enumerate(request(1, "/fixture"), sink().1);
        fake.complete(0, vec![entry("one")]);
        drop(cold);
        clock.set(clock.get() + Duration::from_secs(29));
        let (events, emit) = sink();
        let hit = source.enumerate(request(2, "/fixture"), emit);
        until(context, || finished(&events));
        assert_eq!(fake.calls.borrow().len(), 1);
        clock.set(clock.get() + Duration::from_secs(1));
        source.cache.borrow_mut().prune(clock.get());
        assert_eq!(fake.live_watches(), 0);
        let expired = source.enumerate(request(3, "/fixture"), sink().1);
        assert_eq!(fake.calls.borrow().len(), 2);
        drop((hit, expired));
    });
}

#[test]
fn routed_source_forwards_cold_ui_watch_and_invalidation_but_not_search_watch() {
    let fake = Rc::new(FakeSource::default());
    let routed = crate::adapters::RoutedFileSource {
        local: source(&fake),
        search: crate::adapters::SearchFileSource::new(),
    };
    let load = routed.enumerate(request(1, "/fixture"), sink().1);
    let watch = routed.watch(Location::local("/fixture"), true, Rc::new(|_| {}));
    assert!(watch.is_some());
    assert_eq!(fake.ui_watch.get(), 1);
    assert!(
        routed
            .watch(Location::uri("search://query"), false, Rc::new(|_| {}))
            .is_none()
    );
    fake.complete(0, vec![entry("one")]);
    routed.invalidate(&Location::local("/fixture"));
    assert_eq!(&*fake.invalidated.borrow(), &[Location::local("/fixture")]);
    assert_eq!(fake.live_watches(), 0);
    drop((load, watch));
}

#[test]
fn metadata_changed_during_load_or_after_cancellation_cannot_poison_cache() {
    on_context(|context| {
        let fake = Rc::new(FakeSource::default());
        let source = source(&fake);
        let cold = source.enumerate(request(1, "/fixture"), sink().1);
        fake.complete(0, vec![entry("song.bin")]);
        drop(cold);
        let mut stale = entry("song.bin");
        Rc::make_mut(&mut stale.0).music_tags = Some(MusicTags {
            title: Some("stale".into()),
            ..MusicTags::default()
        });
        fake.emit(
            0,
            DirectoryEvent::MusicMetadata {
                request_id: RequestId(1),
                entries: vec![stale.clone()],
            },
        );
        let (events, emit) = sink();
        let hit = source.enumerate(request(2, "/fixture"), emit);
        until(context, || finished(&events));
        assert!(
            matches!(&events.borrow()[0], DirectoryEvent::Batch { entries, .. } if entries[0].music_tags.is_none())
        );
        source.invalidate(&Location::local("/fixture"));
        let new = source.enumerate(request(3, "/fixture"), sink().1);
        fake.complete(1, vec![entry("song.bin")]);
        fake.change(&Location::local("/fixture"));
        fake.emit(
            1,
            DirectoryEvent::MusicMetadata {
                request_id: RequestId(3),
                entries: vec![stale],
            },
        );
        assert!(!source.cache.borrow().listings[0].valid.get());
        let next = source.enumerate(request(4, "/fixture"), sink().1);
        assert_eq!(fake.calls.borrow().len(), 3);
        drop((hit, new, next));
    });
}

#[test]
fn lru_eviction_during_replay_automatically_reenumerates() {
    on_context(|context| {
        let fake = Rc::new(FakeSource::default());
        let source = CachedFileSource::with_limits(
            fake.clone(),
            Limits {
                directories: 1,
                ..Limits::default()
            },
            Rc::new(Instant::now),
        );
        let cold = source.enumerate(request(1, "/fixture"), sink().1);
        fake.complete(0, vec![entry("one")]);
        drop(cold);
        let (events, emit) = sink();
        let hit = source.enumerate(request(2, "/fixture"), emit);
        let other = source.enumerate(request(3, "/other"), sink().1);
        fake.complete(1, vec![entry("other")]);
        until(context, || fake.calls.borrow().len() == 3);
        assert!(matches!(
            &events.borrow()[0],
            DirectoryEvent::Restarted { .. }
        ));
        fake.complete(2, vec![entry("fresh")]);
        assert!(finished(&events));
        drop((hit, other));
    });
}

#[test]
fn reentrant_cancel_on_reset_never_starts_fallback() {
    on_context(|context| {
        let fake = Rc::new(FakeSource::default());
        let source = source(&fake);
        let cold = source.enumerate(request(1, "/fixture"), sink().1);
        fake.complete(0, vec![entry("one")]);
        drop(cold);
        let holder = Rc::new(RefCell::new(None::<LoadHandle>));
        let cancel = holder.clone();
        let resets = Rc::new(Cell::new(0));
        let received = resets.clone();
        holder.replace(Some(source.enumerate(
            request(2, "/fixture"),
            Rc::new(move |event| {
                assert!(matches!(event, DirectoryEvent::Restarted { .. }));
                received.set(received.get() + 1);
                cancel.take();
            }),
        )));
        source.invalidate(&Location::local("/fixture"));
        until(context, || resets.get() == 1);
        assert_eq!(fake.calls.borrow().len(), 1);
    });
}

#[test]
fn byte_budget_evicts_lru_across_directories_and_snapshot_replaces_accumulator() {
    let fake = Rc::new(FakeSource::default());
    let budget = listing_bytes(&Location::local("/fixture")) + entry_bytes(&entry("one")) + 64;
    let source = CachedFileSource::with_limits(
        fake.clone(),
        Limits {
            bytes: budget,
            ..Limits::default()
        },
        Rc::new(Instant::now),
    );
    let first = source.enumerate(request(1, "/fixture"), sink().1);
    fake.complete(0, vec![entry("one")]);
    drop(first);
    let next = source.enumerate(request(2, "/another"), sink().1);
    fake.emit(
        1,
        DirectoryEvent::Batch {
            request_id: RequestId(2),
            entries: vec![entry("old")],
        },
    );
    assert_eq!(source.cache.borrow().listings.len(), 1);
    fake.emit(
        1,
        DirectoryEvent::Snapshot {
            request_id: RequestId(2),
            entries: vec![entry("new")],
        },
    );
    assert_eq!(
        source.cache.borrow().listings[0].entries,
        vec![entry("new")]
    );
    assert_eq!(fake.live_watches(), 1);
    drop(next);
    assert_eq!(fake.live_watches(), 0);
}

struct CountingLocal {
    enumerations: Cell<usize>,
}

impl FileSource for CountingLocal {
    fn validate_location(&self, location: &Location) -> Result<(), LocationValidationError> {
        crate::adapters::LocalFileSource.validate_location(location)
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        self.enumerations.set(self.enumerations.get() + 1);
        crate::adapters::LocalFileSource.enumerate(request, emit)
    }

    fn watch_invalidation(&self, location: &Location, notify: Rc<dyn Fn()>) -> Option<LoadHandle> {
        crate::adapters::LocalFileSource.watch_invalidation(location, notify)
    }
}

fn measure(
    context: &glib::MainContext,
    source: &dyn FileSource,
    load_request: DirectoryRequest,
) -> (Vec<FileEntry>, Duration, Duration) {
    let started = Instant::now();
    let first = Rc::new(Cell::new(None));
    let complete = Rc::new(Cell::new(None));
    let entries = Rc::new(RefCell::new(Vec::new()));
    let (first_emit, complete_emit, received) = (first.clone(), complete.clone(), entries.clone());
    let load = source.enumerate(
        load_request,
        Rc::new(move |event| match event {
            DirectoryEvent::Batch { entries, .. } => {
                if first_emit.get().is_none() {
                    first_emit.set(Some(started.elapsed()));
                }
                received.borrow_mut().extend(entries);
            }
            DirectoryEvent::Snapshot { entries, .. } => *received.borrow_mut() = entries,
            DirectoryEvent::Restarted { .. } => received.borrow_mut().clear(),
            DirectoryEvent::Finished { .. } => complete_emit.set(Some(started.elapsed())),
            DirectoryEvent::Failed { message, .. } => panic!("{message}"),
            DirectoryEvent::MusicMetadata { .. } | DirectoryEvent::Notice { .. } => {}
        }),
    );
    until(context, || complete.get().is_some());
    drop(load);
    let entries = entries.borrow().clone();
    (
        entries,
        first.get().unwrap_or_default(),
        complete.get().unwrap(),
    )
}

struct TemporaryFolder(std::path::PathBuf);

impl TemporaryFolder {
    fn new() -> Self {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let folder = std::env::temp_dir().join(format!(
            "hermes-directory-cache-{}-{unique}",
            std::process::id()
        ));
        std::fs::create_dir(&folder).unwrap();
        Self(folder)
    }
}

impl Drop for TemporaryFolder {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
#[ignore = "Exclusive default GLib context; generated 10k-file watcher and enumeration timing fixture"]
fn generated_real_watcher_and_warm_enumeration_timing() {
    let context = glib::MainContext::default();
    context.with_thread_default(|| {
        let folder = TemporaryFolder::new();
        for index in 0..10_000 {
            std::fs::write(folder.0.join(format!("entry-{index}.bin")), b"").unwrap();
        }
        let directory = folder.0.to_str().unwrap();
        let (_, raw_first, raw_total) = measure(&context, &crate::adapters::LocalFileSource, request(1, directory));
        let provider = Rc::new(CountingLocal { enumerations: Cell::new(0) });
        let source = CachedFileSource::new(provider.clone());
        let (cold_entries, cold_first, cold_total) = measure(&context, &source, request(2, directory));
        assert_eq!(cold_entries.len(), 10_000);
        assert_eq!(source.cache.borrow().listings.len(), 1);
        let (warm_entries, warm_first, warm_total) = measure(&context, &source, request(3, directory));
        assert_eq!(warm_entries, cold_entries);
        assert_eq!(provider.enumerations.get(), 1);
        eprintln!("generated 10k entries, debug, enumeration delivery only: raw first={raw_first:?} total={raw_total:?}; cached cold first={cold_first:?} total={cold_total:?}; warm first={warm_first:?} total={warm_total:?}; accounted bytes={}", source.cache.borrow().listings[0].bytes);
        std::fs::write(folder.0.join(".new-hidden"), b"generated").unwrap();
        until(&context, || !source.cache.borrow().listings[0].valid.get());
        let (fresh_entries, _, _) = measure(&context, &source, request(4, directory));
        assert_eq!(fresh_entries.len(), 10_000);
        assert_eq!(provider.enumerations.get(), 2);
        std::fs::write(folder.0.join("added.bin"), b"").unwrap();
        until(&context, || !source.cache.borrow().listings[0].valid.get());
        let (added, _, _) = measure(&context, &source, request(5, directory));
        assert_eq!(added.len(), 10_001);
        std::fs::remove_file(folder.0.join("added.bin")).unwrap();
        until(&context, || !source.cache.borrow().listings[0].valid.get());
        let (removed, _, _) = measure(&context, &source, request(6, directory));
        assert_eq!(removed.len(), 10_000);
        source.cache.borrow_mut().limits.ttl = Duration::from_millis(10);
        until(&context, || source.cache.borrow().listings.is_empty());
        drop(source);
    }).unwrap();
    while context.iteration(false) {}
}

#[test]
#[ignore = "Exclusive default GLib context; generated symlink retarget and deleted directory fixture"]
fn generated_real_watcher_preserves_aliases_and_detects_retarget_and_deletion() {
    let context = glib::MainContext::default();
    context
        .with_thread_default(|| {
            let folder = TemporaryFolder::new();
            for name in ["old", "new"] {
                std::fs::create_dir(folder.0.join(name)).unwrap();
                std::fs::write(folder.0.join(name).join(format!("{name}.bin")), b"").unwrap();
            }
            let link = folder.0.join("alias");
            std::os::unix::fs::symlink(folder.0.join("old"), &link).unwrap();
            let provider = Rc::new(CountingLocal {
                enumerations: Cell::new(0),
            });
            let source = CachedFileSource::new(provider.clone());
            let alias = link.to_str().unwrap();
            let (old, _, _) = measure(&context, &source, request(1, alias));
            assert_eq!(old[0].location, Location::local(link.join("old.bin")));
            let (warm, _, _) = measure(&context, &source, request(2, alias));
            assert_eq!(warm, old);
            assert_eq!(provider.enumerations.get(), 1);
            std::os::unix::fs::symlink(folder.0.join("new"), folder.0.join("replacement")).unwrap();
            std::fs::rename(folder.0.join("replacement"), &link).unwrap();
            until(&context, || !source.cache.borrow().listings[0].valid.get());
            let (new, _, _) = measure(&context, &source, request(3, alias));
            assert_eq!(new[0].location, Location::local(link.join("new.bin")));
            assert_eq!(provider.enumerations.get(), 2);
            std::fs::remove_dir_all(folder.0.join("new")).unwrap();
            until(&context, || !source.cache.borrow().listings[0].valid.get());
            let (events, emit) = sink();
            let failed = source.enumerate(request(4, alias), emit);
            until(&context, || {
                events
                    .borrow()
                    .iter()
                    .any(|event| matches!(event, DirectoryEvent::Failed { .. }))
            });
            assert!(!finished(&events));
            drop((failed, source));
        })
        .unwrap();
    while context.iteration(false) {}
}
