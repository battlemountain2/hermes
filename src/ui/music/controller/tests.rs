use super::*;

fn pump(player: &MusicPlayer, predicate: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while !predicate() && std::time::Instant::now() < deadline {
        if !glib::MainContext::default().iteration(false) {
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    assert!(predicate(), "Timed out: {}", player.state.detail.text());
}

#[test]
#[ignore = "Requires a private GTK display, FFmpeg and a built worker; generated WAV only"]
fn desktop_music_seek_controls_follow_stream_and_keep_latest_target() {
    use crate::mpris::{Backend, Command};
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let folder = std::env::temp_dir().join(format!("hermes-music-seek-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join("track.wav");
    assert!(
        std::process::Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-nostdin",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=440",
                "-t",
                "12"
            ])
            .arg(&path)
            .status()
            .unwrap()
            .success()
    );
    let controller = Controller::with_session(None);
    let first = MusicPlayer::with_controller(controller.clone());
    let second = MusicPlayer::with_controller(controller.clone());
    controller.set_volume(0.0);
    let window = gtk::Window::builder().child(&first.bar()).build();
    window.present();
    controller.replace(
        vec![Track {
            path,
            tags: Default::default(),
        }],
        0,
    );
    pump(&first, || first.is_playing());
    controller.command(Command::Pause).unwrap();
    let stream = controller.stream.borrow().clone().unwrap();
    let revision = controller.seek_revision();
    first.state.seek.set_value(3.0);
    assert!(stream.is_seeking());
    assert!(first.state.seek.is_sensitive());
    first.state.seek.set_value(4.0);
    first.state.seek.set_value(7.0);
    assert_eq!(first.state.seek.value(), 7.0);
    pump(&first, || {
        !stream.is_seeking() && stream.timestamp() >= 7_000_000
    });
    assert!(controller.seek_revision() > revision);
    assert!(
        !first.is_playing(),
        "Scrubbing a paused song must not start playback"
    );
    assert!((first.state.seek.value() - second.state.seek.value()).abs() < 0.1);
    assert_eq!(controller.queue.borrow().tracks.len(), 1);
    controller.stop();
    assert!(!first.state.seek.is_sensitive());
    assert_eq!(first.state.seek.value(), 0.0);
    window.close();
    controller.shutdown();
    drop(first);
    drop(second);
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
#[ignore = "Requires desktop, FFmpeg and a built worker; uses generated media and temporary session state"]
fn desktop_application_shared_controller_multiview_shutdown_and_restore() {
    use crate::mpris::{Backend, Command};
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let folder = std::env::temp_dir().join(format!("hermes-shared-music-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let session_path = folder.join("queue.json");
    let _removed = std::fs::remove_file(&session_path);
    let tracks: Vec<_> = (0..3)
        .map(|index| {
            let path = folder.join(format!("{index}.flac"));
            assert!(
                std::process::Command::new("ffmpeg")
                    .args([
                        "-v",
                        "error",
                        "-nostdin",
                        "-y",
                        "-f",
                        "lavfi",
                        "-i",
                        "sine=frequency=440",
                        "-t",
                        "30"
                    ])
                    .arg(&path)
                    .status()
                    .unwrap()
                    .success()
            );
            Track {
                path,
                tags: crate::model::MusicTags {
                    title: Some(format!("Shared fixture {index}")),
                    ..Default::default()
                },
            }
        })
        .collect();
    let app = gtk::Application::builder()
        .application_id("com.strata.SharedMusicFixture")
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    let final_controller = Rc::new(RefCell::new(None));
    let retained = final_controller.clone();
    let session = session_path.clone();
    let fixture_tracks = tracks.clone();
    app.connect_activate(move |app| {
        let controller = Controller::for_application_with_session(app, Some(session.clone()));
        retained.replace(Some(controller.clone()));
        let other_app = gtk::Application::builder()
            .application_id("com.strata.OtherSharedMusicFixture")
            .build();
        let other = Controller::for_application_with_session(&other_app, None);
        assert!(!Rc::ptr_eq(&controller, &other));
        let other_weak = Rc::downgrade(&other);
        drop(other);
        assert!(other_weak.upgrade().is_some());
        drop(other_app);
        assert!(other_weak.upgrade().is_none());
        let first = MusicPlayer::for_application(app);
        let second = MusicPlayer::for_application(app);
        assert!(Rc::ptr_eq(&first.controller(), &second.controller()));
        assert_ne!(first.bar(), second.bar());
        assert_ne!(first.view(), second.view());
        let first_window = gtk::ApplicationWindow::builder()
            .application(app)
            .child(&first.bar())
            .build();
        let second_window = gtk::ApplicationWindow::builder()
            .application(app)
            .child(&second.bar())
            .build();
        first_window.present();
        second_window.present();
        first.state.volume.set_value(0.0);
        first.state.replace(fixture_tracks.clone(), 0);
        pump(&first, || {
            first.is_playing() && second.state.bar.is_visible()
        });
        let stream = controller.stream.borrow().clone().unwrap();
        let play_icon = first.state.play.child().unwrap();
        let speaker_icon = first.state.speaker.child().unwrap();
        controller.notify();
        controller.notify();
        assert_eq!(first.state.play.child().unwrap(), play_icon);
        assert_eq!(first.state.speaker.child().unwrap(), speaker_icon);
        assert_eq!(second.state.title.text(), first.state.title.text());
        assert_eq!(second.state.queue_model.n_items(), 3);
        let id = controller.snapshot().track_id;
        first.state.shuffle.set_active(true);
        first.state.repeat.emit_clicked();
        assert!(second.state.shuffle.is_active());
        assert_eq!(second.state.repeat.label().as_deref(), Some("Repeat: all"));
        assert!(first.state.move_queue(0, 3));
        assert_eq!(controller.snapshot().track_id, id);
        assert_eq!(controller.stream.borrow().as_ref().unwrap(), &stream);
        second.state.volume.set_value(0.2);
        pump(&first, || first.state.volume.value() == 0.2);
        assert_eq!(first.state.volume.value(), 0.2);
        controller.set_volume(0.0);
        let revision = controller.seek_revision();
        first.state.seek.set_value(8.0);
        pump(&first, || {
            !stream.is_seeking()
                && controller.seek_revision() > revision
                && controller.position() >= 8_000_000
        });
        assert!((first.state.seek.value() - second.state.seek.value()).abs() < 0.1);
        controller
            .command(Command::SetPosition("/stale".into(), 25_000_000))
            .unwrap();
        assert!(controller.position() < 15_000_000);
        let revision = controller.seek_revision();
        controller
            .command(Command::SetPosition(id.clone(), controller.duration() + 1))
            .unwrap();
        assert!(!stream.is_seeking());
        assert_eq!(controller.seek_revision(), revision);
        controller.set_shuffle(false);
        controller.command(Command::Pause).unwrap();
        controller.command(Command::Seek(i64::MAX)).unwrap();
        pump(&second, || {
            controller
                .stream
                .borrow()
                .as_ref()
                .is_some_and(|stream| stream.is_prepared())
        });
        assert_eq!(controller.status(), PlaybackStatus::Paused);
        assert!(!second.is_playing());
        assert_eq!(controller.queue.borrow().current, Some(1));
        controller.previous();
        pump(&second, || {
            controller
                .stream
                .borrow()
                .as_ref()
                .is_some_and(|stream| stream.is_prepared())
        });
        assert_eq!(controller.queue.borrow().current, Some(0));
        assert_eq!(controller.status(), PlaybackStatus::Paused);
        let paused = controller.stream.borrow().clone().unwrap();
        let revision = controller.seek_revision();
        second.state.seek.set_value(6.0);
        pump(&second, || {
            !paused.is_seeking()
                && controller.seek_revision() > revision
                && controller.position() >= 6_000_000
        });
        let current_id = controller.track_id();
        let revision = controller.seek_revision();
        controller.previous();
        pump(&second, || {
            !paused.is_seeking()
                && controller.seek_revision() > revision
                && controller.position() < 1_000_000
        });
        assert_eq!(controller.track_id(), current_id);
        assert_eq!(controller.stream.borrow().as_ref().unwrap(), &paused);
        let revision = controller.seek_revision();
        controller.command(Command::Seek(i64::MIN)).unwrap();
        pump(&second, || {
            !paused.is_seeking()
                && controller.seek_revision() > revision
                && controller.position() == 0
        });
        controller.stop();
        controller.next();
        assert_eq!(controller.queue.borrow().current, Some(1));
        assert!(controller.stream.borrow().is_none());
        assert_eq!(controller.status(), PlaybackStatus::Stopped);
        assert!(!first.state.bar.is_visible() && !second.state.bar.is_visible());
        controller.play();
        pump(&second, || second.is_playing());
        let live = controller.stream.borrow().clone().unwrap();
        let first_state = Rc::downgrade(&first.state);
        first.dispose_view();
        first_window.close();
        drop(first_window);
        drop(first);
        assert!(first_state.upgrade().is_none());
        assert_eq!(controller.stream.borrow().as_ref().unwrap(), &live);
        assert!(live.is_playing());
        let third = MusicPlayer::for_application(app);
        let third_window = gtk::ApplicationWindow::builder()
            .application(app)
            .child(&third.bar())
            .build();
        third_window.present();
        assert_eq!(third.state.title.text(), second.state.title.text());
        assert!(third.state.bar.is_visible());
        assert_eq!(third.state.volume.value(), second.state.volume.value());
        assert_eq!(
            third.state.queue_model.n_items(),
            second.state.queue_model.n_items()
        );
        let revision = controller.seek_revision();
        third.state.seek.set_value(28.5);
        pump(&third, || {
            !live.is_seeking() && controller.seek_revision() > revision
        });
        pump(&third, || {
            controller.queue.borrow().current == Some(2) && third.is_playing()
        });
        assert_eq!(second.state.title.text(), third.state.title.text());
        controller.pause();
        let old_identity = controller.track_id();
        let old_object_path = controller.snapshot().track_id;
        controller
            .queue
            .borrow_mut()
            .append(vec![fixture_tracks[2].clone()]);
        controller.queue_changed();
        third.state.play_queue(3);
        pump(&third, || third.is_playing());
        assert_ne!(controller.track_id(), old_identity);
        controller.pause();
        let duplicate = controller.stream.borrow().clone().unwrap();
        let duplicate_identity = controller.track_id();
        let revision = controller.seek_revision();
        controller
            .command(Command::SetPosition(old_object_path, 20_000_000))
            .unwrap();
        assert!(!duplicate.is_seeking());
        assert_eq!(controller.seek_revision(), revision);
        controller.remove_queue(0);
        assert_eq!(controller.stream.borrow().as_ref().unwrap(), &duplicate);
        assert_eq!(controller.track_id(), duplicate_identity);
        let revision = controller.seek_revision();
        third.state.seek.set_value(7.0);
        pump(&third, || {
            !duplicate.is_seeking()
                && controller.seek_revision() > revision
                && controller.position() >= 7_000_000
        });
        second.dispose_view();
        second_window.close();
        assert!(!controller.closed.get());
        assert_eq!(controller.status(), PlaybackStatus::Paused);
        third.dispose_view();
        third_window.close();
    });
    assert_eq!(
        app.run_with_args(&["shared-music-fixture"]),
        glib::ExitCode::SUCCESS
    );
    let controller = final_controller.borrow_mut().take().unwrap();
    assert!(controller.closed.get());
    assert!(controller.stream.borrow().is_none());
    assert!(controller.session_store.borrow().is_none());
    let saved = std::fs::read(&session_path).unwrap();
    let session = crate::music::session::load(&session_path).unwrap().unwrap();
    assert!(session.position >= 7_000_000);
    assert_eq!(session.into_queue().playing().unwrap().path, tracks[2].path);
    controller.shutdown();
    assert_eq!(std::fs::read(&session_path).unwrap(), saved);
    assert!(CONTROLLERS.with(|registry| registry.borrow().is_empty()));
    let restored_app = gtk::Application::builder()
        .application_id("com.strata.SharedMusicRestoreFixture")
        .flags(gtk::gio::ApplicationFlags::NON_UNIQUE)
        .build();
    let session = session_path.clone();
    restored_app.connect_activate(move |app| {
        let controller = Controller::for_application_with_session(app, Some(session.clone()));
        let view = MusicPlayer::for_application(app);
        let window = gtk::ApplicationWindow::builder()
            .application(app)
            .child(&view.bar())
            .build();
        window.present();
        pump(&view, || view.state.queue_model.n_items() == 3);
        assert_eq!(controller.status(), PlaybackStatus::Stopped);
        assert!(controller.stream.borrow().is_none());
        assert!(!view.state.bar.is_visible());
        assert!(controller.position() >= 7_000_000);
        assert_eq!(controller.session_revision.get(), 0);
        view.dispose_view();
        window.close();
    });
    assert_eq!(
        restored_app.run_with_args(&["shared-music-restore-fixture"]),
        glib::ExitCode::SUCCESS
    );
    assert_eq!(std::fs::read(&session_path).unwrap(), saved);
    std::fs::remove_dir_all(folder).unwrap();
}

fn stopped_controller() -> Rc<Controller> {
    let controller = Controller::new(None);
    let mut queue = Queue::default();
    queue.replace(
        (0..3)
            .map(|index| Track {
                path: PathBuf::from(format!("/tmp/{index}.flac")),
                tags: Default::default(),
            })
            .collect(),
        0,
    );
    controller.restore_queue(queue);
    controller
}

#[test]
fn stopped_next_previous_and_repeat_never_create_a_decoder() {
    let controller = stopped_controller();
    let first = controller.track_id();
    controller.next();
    assert_eq!(controller.queue.borrow().current, Some(1));
    assert_ne!(controller.track_id(), first);
    assert_eq!(controller.status(), PlaybackStatus::Stopped);
    assert!(controller.stream.borrow().is_none());
    controller.previous();
    assert_eq!(controller.track_id(), first);
    controller.set_repeat(Repeat::All);
    controller.previous();
    assert_eq!(controller.queue.borrow().current, Some(0));
    controller.next();
    controller.next();
    controller.next();
    assert_eq!(controller.queue.borrow().current, Some(0));
    assert!(controller.stream.borrow().is_none());
    assert_eq!(controller.position(), 0);
}

#[test]
fn stopped_previous_restarts_saved_offset_after_three_seconds() {
    let controller = stopped_controller();
    controller.next();
    let current = controller.track_id();
    controller.resume_position.set(4_000_000);
    let revision = controller.seek_revision();
    controller.previous();
    assert_eq!(controller.track_id(), current);
    assert_eq!(controller.position(), 0);
    assert!(controller.seek_revision() > revision);
    assert!(controller.stream.borrow().is_none());
    controller.resume_position.set(3_000_000);
    controller.previous();
    assert_eq!(controller.queue.borrow().current, Some(0));
}

#[test]
fn controller_settings_and_stale_remote_position_are_validated() {
    use crate::mpris::{Backend, Command};
    let controller = stopped_controller();
    let snapshot = controller.snapshot();
    assert!(glib::variant::ObjectPath::try_from(snapshot.track_id.as_str()).is_ok());
    assert!(snapshot.can_play && snapshot.can_next);
    assert!(!snapshot.can_pause && !snapshot.can_seek);
    assert_eq!(snapshot.status, "Stopped");
    controller.set_volume(0.25);
    controller.set_volume(f64::NAN);
    assert_eq!(controller.volume(), 0.25);
    controller.set_volume(9.0);
    assert_eq!(controller.volume(), 1.0);
    assert!(
        controller
            .command(Command::SetVolume(f64::INFINITY))
            .is_err()
    );
    controller.set_repeat(Repeat::One);
    controller.set_shuffle(true);
    assert_eq!(controller.snapshot().repeat, Repeat::One);
    assert!(controller.snapshot().shuffle);
    controller
        .command(Command::SetPosition("/stale".into(), 9_000_000))
        .unwrap();
    assert_eq!(controller.position(), 0);
    assert!(controller.stream.borrow().is_none());
}

#[test]
fn volume_motion_coalesces_notifications_and_defers_queue_capture() {
    use crate::mpris::Backend;
    let controller = stopped_controller();
    let calls = Rc::new(Cell::new(0));
    let observed = calls.clone();
    let callback: Rc<dyn Fn()> = Rc::new(move || observed.set(observed.get() + 1));
    controller.observe(&callback);
    let queue = controller.queue.borrow_mut();
    for index in 0..10_000 {
        controller.set_volume(f64::from(index % 100) / 100.0);
    }
    controller.set_volume(0.23);
    assert_eq!(calls.get(), 0);
    assert!(controller.session_revision.get() > 0);
    drop(queue);
    assert_eq!(controller.snapshot().volume, 0.23);
    assert!(controller.flush_volume_changes());
    controller.notify();
    assert_eq!(calls.get(), 1);
    assert!(!controller.flush_volume_changes());
    controller.set_volume(0.23);
    controller.set_volume(f64::NAN);
    assert!(!controller.flush_volume_changes());
    assert_eq!(controller.snapshot().volume, 0.23);
}

#[test]
fn debounced_volume_save_and_shutdown_capture_the_newest_value() {
    let folder = std::env::temp_dir().join(format!(
        "hermes-volume-save-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join("queue.json");
    let controller = Controller::new(Some(&path));
    controller.set_volume(0.42);
    assert!(controller.flush_volume_changes());
    assert!(controller.volume_save_pending.get().is_some());
    assert!(!path.exists());
    controller
        .volume_save_pending
        .set(Some(std::time::Instant::now() - Duration::from_millis(300)));
    assert!(!controller.flush_volume_changes());
    assert!(controller.volume_save_pending.get().is_none());
    if let Some(mut store) = controller.session_store.borrow_mut().take() {
        store.finish();
    }
    let saved = crate::music::session::load(&path).unwrap().unwrap();
    assert_eq!(saved.volume, 0.42);
    controller
        .session_store
        .replace(Some(crate::music::session::Store::new(path.clone())));
    controller.set_volume(0.17);
    controller.shutdown();
    assert_eq!(
        crate::music::session::load(&path).unwrap().unwrap().volume,
        0.17
    );
    controller.set_volume(0.99);
    assert_eq!(controller.volume(), 0.17);
    assert!(controller.volume_save_pending.get().is_none());
    assert!(!controller.volume_notify_pending.get());
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
#[ignore = "Requires private GTK display, FFmpeg and a built worker; generated silent WAV only"]
fn desktop_music_volume_motion_keeps_latest_native_multiview_and_remote_value() {
    use crate::mpris::{Backend, Command};
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let folder = std::env::temp_dir().join(format!("hermes-volume-ui-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join("silent.wav");
    assert!(
        std::process::Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-nostdin",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "anullsrc=r=48000:cl=stereo",
                "-t",
                "2",
            ])
            .arg(&path)
            .status()
            .unwrap()
            .success()
    );
    let session_path = folder.join("queue.json");
    let controller = Controller::with_session(Some(session_path.clone()));
    let first = MusicPlayer::with_controller(controller.clone());
    let second = MusicPlayer::with_controller(controller.clone());
    let window = gtk::Window::builder().child(&first.bar()).build();
    window.present();
    controller.set_volume(0.0);
    controller.replace(
        vec![Track {
            path,
            tags: Default::default(),
        }],
        0,
    );
    let stream = controller.stream.borrow().clone().unwrap();
    pump(&first, || stream.is_prepared());
    controller.pause();
    let calls = Rc::new(Cell::new(0));
    let observed = calls.clone();
    let callback: Rc<dyn Fn()> = Rc::new(move || observed.set(observed.get() + 1));
    controller.observe(&callback);
    let queue = controller.queue.borrow_mut();
    for index in 0..10_000 {
        first.state.volume.set_value(f64::from(index % 100) / 100.0);
    }
    first.state.volume.set_value(0.31);
    assert_eq!(calls.get(), 0);
    assert_eq!(controller.volume(), 0.31);
    assert_eq!(stream.volume(), 0.31);
    drop(queue);
    stream.set_muted(true);
    controller.command(Command::SetVolume(0.19)).unwrap();
    assert_eq!(controller.snapshot().volume, 0.19);
    assert!(stream.is_muted());
    stream.set_muted(false);
    pump(&first, || second.state.volume.value() == 0.19);
    assert_eq!(first.state.volume.value(), 0.19);
    assert!(!stream.is_muted());
    assert_eq!(stream.volume(), 0.19);
    assert!(!stream.is_playing());
    assert_eq!(controller.stream.borrow().as_ref(), Some(&stream));
    assert!(calls.get() < 5);
    first.state.volume.set_value(0.11);
    controller.shutdown();
    assert_eq!(
        crate::music::session::load(&session_path)
            .unwrap()
            .unwrap()
            .volume,
        0.11
    );
    window.close();
    first.dispose_view();
    second.dispose_view();
    std::fs::remove_dir_all(folder).unwrap();
}

#[test]
fn observers_are_weak_and_reentrant_changes_do_not_hold_borrows() {
    let controller = stopped_controller();
    let calls = Rc::new(Cell::new(0));
    let observed = calls.clone();
    let weak = Rc::downgrade(&controller);
    let callback: Rc<dyn Fn()> = Rc::new(move || {
        observed.set(observed.get() + 1);
        if let Some(controller) = weak.upgrade()
            && controller.volume() != 0.5
        {
            controller.set_volume(0.5);
        }
    });
    controller.observe(&callback);
    controller.set_repeat(Repeat::All);
    assert_eq!(controller.volume(), 0.5);
    assert_eq!(calls.get(), 2);
    drop(callback);
    controller.set_volume(0.25);
    assert_eq!(calls.get(), 2);
    let weak = Rc::downgrade(&controller);
    drop(controller);
    assert!(weak.upgrade().is_none());
}

#[test]
fn controller_shutdown_is_idempotent_and_rejects_commands() {
    use crate::mpris::{Backend, Command};
    let controller = stopped_controller();
    controller.shutdown();
    let revision = controller.seek_revision();
    controller.shutdown();
    controller.stop();
    assert!(controller.closed.get());
    assert_eq!(controller.seek_revision(), revision);
    assert!(controller.command(Command::Play).is_err());
    assert!(!controller.capabilities().can_play);
}

#[test]
fn occurrence_identity_survives_order_changes_and_prior_removal() {
    let mut identities = TrackIdentity::default();
    identities.replace(4);
    let current = identities.current(Some(2));
    identities.extend(6);
    assert_eq!(identities.current(Some(2)), current);
    identities.remove(0);
    assert_eq!(identities.current(Some(1)), current);
}

#[test]
fn replacement_and_duplicate_occurrences_have_distinct_identities() {
    let mut identities = TrackIdentity::default();
    identities.replace(2);
    let first = identities.current(Some(0));
    let duplicate = identities.current(Some(1));
    assert_ne!(first, duplicate);
    identities.replace(2);
    assert_ne!(identities.current(Some(0)), first);
    assert_ne!(identities.current(Some(1)), duplicate);
    assert_eq!(identities.current(None), None);
    assert_eq!(identities.current(Some(99)), None);
}

#[test]
fn removing_current_duplicate_changes_identity_even_at_same_index() {
    let mut identities = TrackIdentity::default();
    identities.replace(3);
    let before = identities.current(Some(1));
    identities.remove(1);
    assert_ne!(identities.current(Some(1)), before);
}

#[test]
fn queue_reordering_does_not_change_track_object_path() {
    let mut queue = Queue::default();
    let duplicate = Track {
        path: PathBuf::from("/tmp/duplicate.flac"),
        tags: Default::default(),
    };
    queue.replace(vec![duplicate.clone(), duplicate.clone(), duplicate], 1);
    let mut identities = TrackIdentity::default();
    identities.replace(queue.tracks.len());
    let id = identities.current(queue.current);
    assert!(queue.move_before(1, 0));
    queue.set_shuffle(true, 42);
    assert_eq!(identities.current(queue.current), id);
    queue.remove(0);
    identities.remove(0);
    assert_eq!(identities.current(queue.current), id);
    queue.select(1);
    assert_ne!(identities.current(queue.current), id);
}
