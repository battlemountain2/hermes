use super::*;
use std::{
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

#[test]
#[ignore = "GTK/private bus/built worker integration; only generated silent media, no personal state"]
fn desktop_private_bus_drives_real_shared_music_controller() {
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let folder = std::env::temp_dir().join(format!("hermes-mpris-playback-{}", std::process::id()));
    std::fs::create_dir_all(&folder).unwrap();
    let path = folder.join("fixture.flac");
    assert!(
        Process::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-nostdin",
                "-y",
                "-f",
                "lavfi",
                "-i",
                "anullsrc=r=44100:cl=stereo",
                "-t",
                "30"
            ])
            .arg(&path)
            .status()
            .unwrap()
            .success()
    );
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
    let context = glib::MainContext::default();
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
            let controller = crate::ui::test_music_controller();
            controller.set_volume(0.0);
            let backend: Rc<dyn Backend> = Rc::new(controller.clone());
            let export = Export::new(
                &server,
                backend.clone(),
                Rc::new(|| {}),
                Rc::new(|| {}),
                false,
            )
            .unwrap();
            let call = |method: &str, parameters: glib::Variant| {
                context
                    .block_on(client.call_future(
                        server.unique_name().as_deref(),
                        PATH,
                        PLAYER,
                        method,
                        Some(&parameters),
                        None,
                        gio::DBusCallFlags::NONE,
                        3000,
                    ))
                    .unwrap()
            };
            let pump = |predicate: &dyn Fn() -> bool| {
                let deadline = std::time::Instant::now() + Duration::from_secs(15);
                while !predicate() && std::time::Instant::now() < deadline {
                    context.iteration(false);
                    std::thread::sleep(Duration::from_millis(5));
                }
                assert!(predicate(), "Playback state: {}", backend.snapshot().status);
            };
            assert_eq!(backend.snapshot().status, "Stopped");
            call(
                "OpenUri",
                (gio::File::for_path(&path).uri().as_str(),).to_variant(),
            );
            pump(&|| backend.snapshot().can_seek && backend.snapshot().position > 0);
            let snapshot = backend.snapshot();
            assert_eq!(snapshot.status, "Playing");
            assert_eq!(snapshot.track.as_ref().unwrap().path, path);
            assert!(!snapshot.can_next);
            call("Next", ().to_variant());
            assert_eq!(backend.snapshot().status, "Playing");
            call("Pause", ().to_variant());
            pump(&|| backend.snapshot().status == "Paused");
            let revision = backend.snapshot().seek_revision;
            call(
                "SetPosition",
                (
                    glib::variant::ObjectPath::try_from("/stale").unwrap(),
                    20_000_000_i64,
                )
                    .to_variant(),
            );
            assert_eq!(backend.snapshot().seek_revision, revision);
            call(
                "SetPosition",
                (
                    glib::variant::ObjectPath::try_from(snapshot.track_id.as_str()).unwrap(),
                    5_000_000_i64,
                )
                    .to_variant(),
            );
            pump(&|| {
                backend.snapshot().seek_revision > revision
                    && backend.snapshot().position >= 5_000_000
            });
            assert_eq!(backend.snapshot().status, "Paused");
            let revision = backend.snapshot().seek_revision;
            call("Seek", (-2_000_000_i64,).to_variant());
            pump(&|| {
                backend.snapshot().seek_revision > revision
                    && backend.snapshot().position <= 3_200_000
            });
            call("Play", ().to_variant());
            pump(&|| backend.snapshot().status == "Playing");
            assert_eq!(backend.snapshot().track_id, snapshot.track_id);
            assert!(backend.snapshot().position >= 3_000_000);
            call("Stop", ().to_variant());
            assert_eq!(backend.snapshot().status, "Stopped");
            assert!(!backend.snapshot().can_seek);
            call("Play", ().to_variant());
            pump(&|| backend.snapshot().can_seek && backend.snapshot().position > 0);
            assert!(backend.snapshot().position < 2_000_000);
            controller.shutdown();
            drop(export);
            client.close_sync(None::<&gio::Cancellable>).unwrap();
            server.close_sync(None::<&gio::Cancellable>).unwrap();
        })
        .unwrap();
    std::fs::remove_dir_all(folder).unwrap();
}
