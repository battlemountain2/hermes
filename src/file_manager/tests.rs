use super::*;

fn call(method: &str, uris: &[&str]) -> Vec<Request> {
    requests(
        method,
        &uris.iter().map(|uri| uri.to_string()).collect::<Vec<_>>(),
        "token",
    )
    .unwrap()
}

#[test]
fn item_requests_group_parents_preserve_order_and_deduplicate_selection() {
    let requests = call(
        "ShowItems",
        &[
            "file:///tmp/one/second%20song.flac",
            "file:///tmp/two/photo.jpg",
            "file:///tmp/one/first.flac",
            "file:///tmp/one/first.flac",
        ],
    );
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].directory, Location::local("/tmp/one"));
    assert_eq!(
        requests[0].selection,
        [
            Location::local("/tmp/one/second song.flac"),
            Location::local("/tmp/one/first.flac")
        ]
    );
    assert_eq!(requests[0].startup_id, "token");
    assert!(!requests[0].properties);
    assert_eq!(requests[1].directory, Location::local("/tmp/two"));
}

#[test]
fn folders_roots_and_remote_reveals_do_not_need_synchronous_filesystem_io() {
    let folders = call(
        "ShowFolders",
        &[
            "file:///not-mounted/folder",
            "file:///not-mounted/folder",
            "file:///",
        ],
    );
    assert_eq!(folders.len(), 2);
    assert!(folders.iter().all(|request| request.selection.is_empty()));
    assert_eq!(folders[1].directory, Location::local("/"));
    let root = call("ShowItems", &["file:///"]);
    assert_eq!(root[0].directory, Location::local("/"));
    assert!(root[0].selection.is_empty());
    let remote = call("ShowItems", &["sftp://host/share/album/song.flac"]);
    assert_eq!(
        remote[0].directory.uri_value(),
        Some("sftp://host/share/album")
    );
    assert_eq!(
        remote[0].selection[0].uri_value(),
        Some("sftp://host/share/album/song.flac")
    );
    assert_eq!(
        call("ShowFolders", &["file://localhost/tmp/a"])[0].directory,
        Location::local("/tmp/a")
    );
}

#[test]
fn properties_get_a_separate_existing_properties_view_for_each_requested_item() {
    let properties = call(
        "ShowItemProperties",
        &["file:///tmp/a.txt", "file:///tmp/b.txt"],
    );
    assert_eq!(properties.len(), 2);
    assert!(
        properties
            .iter()
            .all(|request| request.properties && request.selection.len() == 1)
    );
    assert_eq!(properties[1].selection[0], Location::local("/tmp/b.txt"));
}

#[test]
fn encoded_native_names_remain_byte_safe_without_accepting_nul() {
    use std::os::unix::ffi::OsStrExt;
    let parsed = call("ShowItems", &["file:///tmp/byte%FF.flac"]);
    assert_eq!(
        parsed[0].selection[0]
            .native_path()
            .unwrap()
            .as_os_str()
            .as_bytes(),
        b"/tmp/byte\xff.flac"
    );
    assert!(requests("ShowItems", &["file:///tmp/bad%00name".into()], "").is_err());
}

#[test]
fn invalid_requests_are_rejected_atomically_with_bounded_fanout() {
    for uri in [
        "relative/path",
        "file:relative",
        "https://example.com/file",
        "file://server/share/file",
        "file:///tmp/a?query",
        "file:///tmp/a#fragment",
        "file:///tmp/bad\nname",
        "file:///tmp/%ZZ",
    ] {
        assert!(
            requests("ShowItems", &["file:///tmp/valid".into(), uri.into()], "").is_err(),
            "{uri}"
        );
    }
    assert!(requests("ShowFolders", &[], "").is_err());
    assert!(requests("Unknown", &["file:///".into()], "").is_err());
    assert!(requests("ShowItems", &vec!["file:///tmp/a".into(); 65], "").is_err());
    assert!(requests("ShowItems", &["file:///tmp/a".into()], "bad\0token").is_err());
    let windows = (0..17)
        .map(|index| format!("file:///parent-{index}/item"))
        .collect::<Vec<_>>();
    assert!(requests("ShowItems", &windows, "").is_err());
    assert!(
        requests(
            "ShowFolders",
            &[format!("file:///{}", "x".repeat(17 * 1024))],
            ""
        )
        .is_err()
    );
}

#[test]
fn folder_integration_defaults_off_and_rejects_unsafe_or_unrecognized_preferences() {
    let directory = crate::sandbox::browser::format_tests::FixtureDirectory::new();
    let path = directory.0.join("preference.json");
    assert!(!enabled_at(&path));
    for bytes in [
        b"broken".as_slice(),
        br#"{"version":2,"enabled":true}"#,
        br#"{"version":1,"enabled":false}"#,
        br#"{"version":1,"enabled":true,"extra":1}"#,
    ] {
        fs::write(&path, bytes).unwrap();
        assert!(!enabled_at(&path));
    }
    fs::write(&path, br#"{"version":1,"enabled":true}"#).unwrap();
    assert!(enabled_at(&path));
    let alias = directory.0.join("alias.json");
    std::os::unix::fs::symlink(&path, &alias).unwrap();
    assert!(!enabled_at(&alias));
    fs::write(&path, vec![b' '; 1025]).unwrap();
    assert!(!enabled_at(&path));
    assert!(!enabled_at(&directory.0));
}

#[test]
fn introspection_has_only_the_three_standard_methods() {
    let node = gio::DBusNodeInfo::for_xml(XML).unwrap();
    let interface = node.lookup_interface(BUS).unwrap();
    assert_eq!(XML.matches("<method ").count(), 3);
    for name in ["ShowFolders", "ShowItems", "ShowItemProperties"] {
        assert!(interface.lookup_method(name).is_some());
    }
    assert!(interface.lookup_method("OpenFile").is_none());
}

#[test]
#[ignore = "Starts its own private dbus-daemon; no desktop connection, GUI or audio"]
fn private_bus_dispatches_requests_keeps_existing_owner_and_releases_registrations() {
    use std::{
        io::{BufRead, BufReader},
        process::{Child, Command, Stdio},
        time::{Duration, Instant},
    };
    struct Bus(Child);
    impl Drop for Bus {
        fn drop(&mut self) {
            let _killed = self.0.kill();
            let _waited = self.0.wait();
        }
    }
    let mut bus = Bus(Command::new("dbus-daemon")
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
            let connect = || {
                gio::DBusConnection::for_address_sync(
                    address.trim(),
                    gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
                        | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
                    None,
                    None::<&gio::Cancellable>,
                )
                .unwrap()
            };
            let server = connect();
            let client = connect();
            let competitor = connect();
            for connection in [&server, &client, &competitor] {
                connection.set_exit_on_close(false);
            }
            let received = Rc::new(RefCell::new(Vec::new()));
            let captured = received.clone();
            let first = Owner::start(
                &server,
                Rc::new(move |request| captured.borrow_mut().push(request)),
            );
            let until = |condition: &dyn Fn() -> bool| {
                let started = Instant::now();
                while !condition() {
                    assert!(started.elapsed() < Duration::from_secs(3));
                    context.iteration(false);
                    std::thread::sleep(Duration::from_millis(2));
                }
            };
            until(&|| first.export.borrow().is_some());
            let losing = Owner::start(
                &competitor,
                Rc::new(|_| panic!("Existing owner was replaced")),
            );
            let dbus = |method: &str, parameters: glib::Variant| {
                context.block_on(client.call_future(
                    Some("org.freedesktop.DBus"),
                    "/org/freedesktop/DBus",
                    "org.freedesktop.DBus",
                    method,
                    Some(&parameters),
                    None,
                    gio::DBusCallFlags::NONE,
                    2000,
                ))
            };
            assert_eq!(
                dbus("GetNameOwner", (BUS,).to_variant())
                    .unwrap()
                    .get::<(String,)>()
                    .unwrap()
                    .0,
                server.unique_name().unwrap()
            );
            let invoke = |method: &str, uris: Vec<String>| {
                context.block_on(client.call_future(
                    Some(BUS),
                    PATH,
                    BUS,
                    method,
                    Some(&(uris, "private-token").to_variant()),
                    None,
                    gio::DBusCallFlags::NONE,
                    2000,
                ))
            };
            for method in ["ShowFolders", "ShowItems", "ShowItemProperties"] {
                invoke(method, vec!["file:///tmp/generated/song.flac".into()]).unwrap();
            }
            assert_eq!(received.borrow().len(), 3);
            assert!(received.borrow()[2].properties);
            assert!(invoke("ShowItems", vec!["https://example.com/bad".into()]).is_err());
            assert_eq!(received.borrow().len(), 3);
            assert!(losing.export.borrow().is_none());
            drop(first);
            until(&|| {
                !dbus("NameHasOwner", (BUS,).to_variant())
                    .unwrap()
                    .get::<(bool,)>()
                    .unwrap()
                    .0
            });
            assert!(losing.export.borrow().is_none());
            assert!(
                context
                    .block_on(client.call_future(
                        server.unique_name().as_deref(),
                        PATH,
                        BUS,
                        "ShowFolders",
                        Some(&(vec!["file:///tmp"], "").to_variant()),
                        None,
                        gio::DBusCallFlags::NONE,
                        2000
                    ))
                    .is_err()
            );
            drop(losing);
            for connection in [&server, &client, &competitor] {
                connection.close_sync(None::<&gio::Cancellable>).unwrap();
            }
        })
        .unwrap();
}
