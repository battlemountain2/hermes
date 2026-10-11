use super::*;
use crate::model::MusicTags;

fn snapshot() -> Snapshot {
    Snapshot {
        track: Some(Track {
            path: PathBuf::from("/tmp/album/song.flac"),
            tags: MusicTags {
                title: Some("Song".into()),
                artist: Some("Artist".into()),
                album: Some("Album".into()),
                track: Some(3),
                ..Default::default()
            },
        }),
        track_id: "/org/mpris/MediaPlayer2/track/one".into(),
        status: "Paused",
        position: 3_000_000,
        duration: 60_000_000,
        volume: 0.5,
        repeat: Repeat::Off,
        shuffle: false,
        can_next: true,
        can_previous: true,
        can_play: true,
        can_pause: true,
        can_seek: true,
        seek_revision: 0,
        art_url: None,
    }
}

#[test]
fn metadata_has_standard_types_and_is_deterministic_without_position_updates() {
    let state = snapshot();
    let properties = player_properties(&state);
    let metadata = properties["Metadata"]
        .get::<BTreeMap<String, glib::Variant>>()
        .unwrap();
    assert_eq!(metadata["mpris:trackid"].type_().as_str(), "o");
    assert_eq!(metadata["mpris:length"].get::<i64>(), Some(60_000_000));
    assert_eq!(
        metadata["xesam:artist"].get::<Vec<String>>(),
        Some(vec!["Artist".into()])
    );
    assert_eq!(metadata["xesam:trackNumber"].get::<i32>(), Some(3));
    assert!(!properties.contains_key("Position"));
    for _ in 0..50 {
        assert_eq!(player_properties(&state), properties);
    }
    let mut stopped = state;
    stopped.track = None;
    assert_eq!(player_properties(&stopped)["Metadata"].n_children(), 0);
}

#[test]
fn writable_values_reject_invalid_types_and_nonfinite_volume() {
    assert_eq!(
        parse_property("Volume", &(-0.5_f64).to_variant()),
        Ok(Command::SetVolume(0.0))
    );
    assert_eq!(
        parse_property("Volume", &(2_f64).to_variant()),
        Ok(Command::SetVolume(1.0))
    );
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(parse_property("Volume", &value.to_variant()).is_err());
    }
    assert_eq!(
        parse_property("LoopStatus", &"Playlist".to_variant()),
        Ok(Command::SetRepeat(Repeat::All))
    );
    assert_eq!(
        parse_property("Rate", &(0_f64).to_variant()),
        Ok(Command::Pause)
    );
    assert!(parse_property("LoopStatus", &"invalid".to_variant()).is_err());
    assert!(parse_property("Shuffle", &"true".to_variant()).is_err());
    assert!(parse_property("Position", &42_i64.to_variant()).is_err());
}

#[test]
fn methods_validate_argument_signatures_and_only_allow_local_audio_uris() {
    assert_eq!(parse_method("Play", &().to_variant()), Ok(Command::Play));
    assert_eq!(
        parse_method("Seek", &(-1_000_000_i64,).to_variant()),
        Ok(Command::Seek(-1_000_000))
    );
    assert!(parse_method("Seek", &(1_i32,).to_variant()).is_err());
    let track = glib::variant::ObjectPath::try_from("/track/one").unwrap();
    assert_eq!(
        parse_method("SetPosition", &(track, 12_i64).to_variant()),
        Ok(Command::SetPosition("/track/one".into(), 12))
    );
    assert_eq!(
        local_audio_uri("file:///tmp/a%20song.FLAC"),
        Some(PathBuf::from("/tmp/a song.FLAC"))
    );
    for uri in [
        "https://example.com/song.mp3",
        "file://server/music/song.flac",
        "file:///tmp/run.desktop",
        "/tmp/song.flac",
        "file:///tmp/movie.mp4",
    ] {
        assert!(local_audio_uri(uri).is_none(), "{uri}");
    }
}

#[test]
fn advertised_property_signatures_match_implementation() {
    let node = gio::DBusNodeInfo::for_xml(include_str!("interfaces.xml")).unwrap();
    assert_eq!(node.interfaces().len(), 2);
    assert_eq!(
        root_properties()["SupportedUriSchemes"].type_().as_str(),
        "as"
    );
    assert_eq!(
        player_properties(&snapshot())["Metadata"].type_().as_str(),
        "a{sv}"
    );
    assert_eq!(
        player_properties(&snapshot())["Volume"].type_().as_str(),
        "d"
    );
}

#[test]
#[ignore = "Private dbus-daemon integration; explicitly run without touching desktop players"]
fn private_bus_exports_controls_properties_signals_and_cleanup() {
    use std::{
        cell::Cell,
        io::{BufRead, BufReader},
        process::{Child, Command as Process, Stdio},
    };
    struct Bus(Child);
    impl Drop for Bus {
        fn drop(&mut self) {
            let _killed = self.0.kill();
            let _waited = self.0.wait();
        }
    }
    struct Fake {
        state: RefCell<Snapshot>,
        commands: RefCell<Vec<Command>>,
    }
    impl Backend for Fake {
        fn snapshot(&self) -> Snapshot {
            self.state.borrow().clone()
        }
        fn command(&self, command: Command) -> Result<(), String> {
            match &command {
                Command::Play => self.state.borrow_mut().status = "Playing",
                Command::Pause => self.state.borrow_mut().status = "Paused",
                Command::SetVolume(volume) => self.state.borrow_mut().volume = *volume,
                _ => {}
            }
            self.commands.borrow_mut().push(command);
            Ok(())
        }
    }
    let mut bus = Bus(Process::new("dbus-daemon")
        .args(["--session", "--nofork", "--print-address=1"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap());
    let mut address = String::new();
    BufReader::new(bus.0.stdout.take().unwrap())
        .read_line(&mut address)
        .unwrap();
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            let flags = gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
                | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION;
            let server = gio::DBusConnection::for_address_sync(
                address.trim(),
                flags,
                None,
                None::<&gio::Cancellable>,
            )
            .unwrap();
            let client = gio::DBusConnection::for_address_sync(
                address.trim(),
                flags,
                None,
                None::<&gio::Cancellable>,
            )
            .unwrap();
            server.set_exit_on_close(false);
            client.set_exit_on_close(false);
            let fake = Rc::new(Fake {
                state: RefCell::new(snapshot()),
                commands: RefCell::new(Vec::new()),
            });
            let raised = Rc::new(Cell::new(0));
            let raise_count = raised.clone();
            let export = Export::new(
                &server,
                fake.clone(),
                Rc::new(move || raise_count.set(raise_count.get() + 1)),
                Rc::new(|| {}),
                false,
            )
            .unwrap();
            let signals = Rc::new(RefCell::new(Vec::new()));
            let captured = signals.clone();
            let subscription = client.subscribe_to_signal(
                Some(server.unique_name().unwrap().as_str()),
                None,
                None,
                Some(PATH),
                None,
                gio::DBusSignalFlags::NONE,
                move |signal| {
                    captured
                        .borrow_mut()
                        .push((signal.signal_name.to_owned(), signal.parameters.clone()))
                },
            );
            let call = |interface: &str, method: &str, parameters: glib::Variant| {
                context.block_on(client.call_future(
                    server.unique_name().as_deref(),
                    PATH,
                    interface,
                    method,
                    Some(&parameters),
                    None,
                    gio::DBusCallFlags::NONE,
                    2000,
                ))
            };
            let all = call(
                "org.freedesktop.DBus.Properties",
                "GetAll",
                (PLAYER,).to_variant(),
            )
            .unwrap();
            let (properties,) = all.get::<(BTreeMap<String, glib::Variant>,)>().unwrap();
            assert_eq!(properties["PlaybackStatus"].str(), Some("Paused"));
            assert_eq!(properties["Position"].get::<i64>(), Some(3_000_000));
            call(PLAYER, "Play", ().to_variant()).unwrap();
            call(PLAYER, "Pause", ().to_variant()).unwrap();
            call(
                "org.freedesktop.DBus.Properties",
                "Set",
                (PLAYER, "Volume", 0.7_f64.to_variant()).to_variant(),
            )
            .unwrap();
            assert_eq!(fake.state.borrow().volume, 0.7);
            call(
                "org.freedesktop.DBus.Properties",
                "Set",
                (PLAYER, "Rate", 1_f64.to_variant()).to_variant(),
            )
            .unwrap();
            assert_eq!(fake.state.borrow().volume, 0.7);
            assert!(
                call(
                    "org.freedesktop.DBus.Properties",
                    "Set",
                    (PLAYER, "Rate", 2_f64.to_variant()).to_variant()
                )
                .is_err()
            );
            assert!(
                call(
                    "org.freedesktop.DBus.Properties",
                    "Set",
                    (PLAYER, "Position", 42_i64.to_variant()).to_variant()
                )
                .is_err()
            );
            assert!(
                call(
                    PLAYER,
                    "OpenUri",
                    ("https://example.com/song.mp3",).to_variant()
                )
                .is_err()
            );
            call(ROOT, "Raise", ().to_variant()).unwrap();
            assert_eq!(raised.get(), 1);
            call(PLAYER, "Seek", (-5_i64,).to_variant()).unwrap();
            call(
                PLAYER,
                "SetPosition",
                (
                    glib::variant::ObjectPath::try_from("/stale").unwrap(),
                    6_i64,
                )
                    .to_variant(),
            )
            .unwrap();
            assert!(fake.commands.borrow().contains(&Command::Seek(-5)));
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            while signals.borrow().len() < 3 && std::time::Instant::now() < deadline {
                context.iteration(false);
            }
            assert!(
                signals
                    .borrow()
                    .iter()
                    .any(|(name, _)| name == "PropertiesChanged")
            );
            signals.borrow_mut().clear();
            for _ in 0..5 {
                export.publish();
            }
            fake.state.borrow_mut().position += 2_000_000;
            export.publish();
            call(
                "org.freedesktop.DBus.Properties",
                "Get",
                (PLAYER, "Position").to_variant(),
            )
            .unwrap();
            while context.pending() {
                context.iteration(false);
            }
            assert!(
                signals.borrow().is_empty(),
                "Normal progress must not emit PropertiesChanged/Seeked"
            );
            fake.state.borrow_mut().seek_revision += 1;
            export.publish();
            let deadline = std::time::Instant::now() + Duration::from_secs(2);
            while signals.borrow().is_empty() && std::time::Instant::now() < deadline {
                context.iteration(false);
            }
            assert_eq!(signals.borrow()[0].0, "Seeked");
            assert_eq!(signals.borrow()[0].1.get::<(i64,)>(), Some((5_000_000,)));
            drop(export);
            assert!(call(PLAYER, "Play", ().to_variant()).is_err());
            drop(subscription);
            client.close_sync(None::<&gio::Cancellable>).unwrap();
            server.close_sync(None::<&gio::Cancellable>).unwrap();
        })
        .unwrap();
}
