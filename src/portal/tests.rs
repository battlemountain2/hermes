use super::*;

#[track_caller]
fn until(condition: &dyn Fn() -> bool) {
    let started = std::time::Instant::now();
    let context = glib::MainContext::default();
    while !condition() {
        assert!(
            started.elapsed() < std::time::Duration::from_secs(8),
            "Timed out waiting for private backend"
        );
        context.iteration(false);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

#[test]
fn backend_exposes_only_chooser_methods_request_close_and_version() {
    let node = gio::DBusNodeInfo::for_xml(XML).unwrap();
    let chooser = node.lookup_interface(INTERFACE).unwrap();
    for method in ["OpenFile", "SaveFile", "SaveFiles"] {
        assert!(chooser.lookup_method(method).is_some());
    }
    assert!(chooser.lookup_property("version").is_some());
    assert!(chooser.lookup_method("ShowFolders").is_none());
    assert!(
        node.lookup_interface("org.freedesktop.impl.portal.Request")
            .unwrap()
            .lookup_method("Close")
            .is_some()
    );
    assert_eq!(XML.matches("<method ").count(), 4);
}

#[test]
#[ignore = "Private GTK display and fresh D-Bus only; exercises generated files, not the real desktop portal"]
#[expect(
    deprecated,
    reason = "The backend tests its direct non-recursive GTK chooser"
)]
fn private_bus_authenticates_frontend_closes_requests_and_returns_selected_files() {
    use std::{
        io::{BufRead, BufReader},
        process::{Child, Command, Stdio},
    };
    gtk::init().unwrap();
    glib::set_application_name(crate::identity::NAME);
    crate::assets::prepare().unwrap();
    let _theme = crate::ui::picker::prepare();
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
    let context = glib::MainContext::default();
    let _guard = context.acquire().unwrap();
    let connect = || {
        let connection = gio::DBusConnection::for_address_sync(
            address.trim(),
            gio::DBusConnectionFlags::AUTHENTICATION_CLIENT
                | gio::DBusConnectionFlags::MESSAGE_BUS_CONNECTION,
            None,
            None::<&gio::Cancellable>,
        )
        .unwrap();
        connection.set_exit_on_close(false);
        connection
    };
    let server = connect();
    let frontend = connect();
    let intruder = connect();
    let application = gtk::Application::builder()
        .application_id("io.github.battlemountain2.Hermes.PortalTest")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    application.register(None::<&gio::Cancellable>).unwrap();
    let export = Export::new(&server, &application).unwrap();
    let invoke = |client: &gio::DBusConnection,
                  path: &str,
                  interface: &str,
                  method: &str,
                  arguments: glib::Variant| {
        let result = Rc::new(RefCell::new(None));
        let captured = result.clone();
        client.call(
            server.unique_name().as_deref(),
            path,
            interface,
            method,
            Some(&arguments),
            None,
            gio::DBusCallFlags::NONE,
            10_000,
            None::<&gio::Cancellable>,
            move |reply| {
                captured.replace(Some(reply));
            },
        );
        result
    };
    let arguments = |suffix: &str| {
        (
            glib::variant::ObjectPath::try_from(format!(
                "/org/freedesktop/portal/desktop/request/generated/{suffix}"
            ))
            .unwrap(),
            "generated.app",
            "",
            "Private chooser",
            Options::new(),
        )
            .to_variant()
    };
    let denied = invoke(&intruder, PATH, INTERFACE, "OpenFile", arguments("denied"));
    until(&|| denied.borrow().is_some());
    assert_eq!(
        denied
            .borrow_mut()
            .take()
            .unwrap()
            .unwrap()
            .get::<(u32, Options)>()
            .unwrap()
            .0,
        2
    );
    assert!(application.windows().is_empty());
    frontend
        .call_sync(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "RequestName",
            Some(&(FRONTEND, 4_u32).to_variant()),
            None,
            gio::DBusCallFlags::NONE,
            2000,
            None::<&gio::Cancellable>,
        )
        .unwrap();
    let response = invoke(&frontend, PATH, INTERFACE, "OpenFile", arguments("cancel"));
    let handle = "/org/freedesktop/portal/desktop/request/generated/cancel";
    until(&|| {
        export
            .pending
            .borrow()
            .get(handle)
            .is_some_and(|pending| pending.picker.is_some())
    });
    let duplicate = invoke(&frontend, PATH, INTERFACE, "OpenFile", arguments("cancel"));
    until(&|| duplicate.borrow().is_some());
    assert!(duplicate.borrow_mut().take().unwrap().is_err());
    let denied_close = invoke(
        &intruder,
        handle,
        "org.freedesktop.impl.portal.Request",
        "Close",
        ().to_variant(),
    );
    until(&|| denied_close.borrow().is_some());
    assert!(denied_close.borrow_mut().take().unwrap().is_err());
    assert_eq!(export.pending.borrow().len(), 1);
    let close = invoke(
        &frontend,
        handle,
        "org.freedesktop.impl.portal.Request",
        "Close",
        ().to_variant(),
    );
    until(&|| close.borrow().is_some() && response.borrow().is_some());
    close.borrow_mut().take().unwrap().unwrap();
    assert_eq!(
        response
            .borrow_mut()
            .take()
            .unwrap()
            .unwrap()
            .get::<(u32, Options)>()
            .unwrap()
            .0,
        1
    );
    assert!(export.pending.borrow().is_empty());
    assert!(application.windows().is_empty());
    let fixture = crate::sandbox::browser::format_tests::FixtureDirectory::new();
    let target = fixture.0.join("chosen file.txt");
    std::fs::write(&target, b"generated").unwrap();
    let (path, app_id, parent, title, mut options) = arguments("success")
        .get::<(glib::variant::ObjectPath, String, String, String, Options)>()
        .unwrap();
    options.insert(
        "current_folder".into(),
        [fixture.0.as_os_str().as_encoded_bytes(), &[0]]
            .concat()
            .to_variant(),
    );
    let response = invoke(
        &frontend,
        PATH,
        INTERFACE,
        "OpenFile",
        (path, app_id, parent, title, options).to_variant(),
    );
    let handle = "/org/freedesktop/portal/desktop/request/generated/success";
    until(&|| {
        export
            .pending
            .borrow()
            .get(handle)
            .is_some_and(|pending| pending.picker.is_some())
    });
    let dialog = application
        .windows()
        .into_iter()
        .find_map(|window| window.downcast::<gtk::FileChooserDialog>().ok())
        .unwrap();
    until(&|| {
        dialog
            .current_folder()
            .and_then(|file| file.path())
            .as_ref()
            == Some(&fixture.0)
    });
    dialog.set_file(&gio::File::for_path(&target)).unwrap();
    until(&|| dialog.file().and_then(|file| file.path()).as_ref() == Some(&target));
    dialog.response(gtk::ResponseType::Accept);
    until(&|| response.borrow().is_some());
    let (status, results) = response
        .borrow_mut()
        .take()
        .unwrap()
        .unwrap()
        .get::<(u32, Options)>()
        .unwrap();
    assert_eq!(status, 0);
    assert_eq!(
        results["uris"].get::<Vec<String>>().unwrap(),
        [gio::File::for_path(&target).uri().to_string()]
    );
    assert!(application.windows().is_empty());
    let response = invoke(
        &frontend,
        PATH,
        INTERFACE,
        "OpenFile",
        arguments("disconnect"),
    );
    until(&|| !application.windows().is_empty());
    frontend.close_sync(None::<&gio::Cancellable>).unwrap();
    until(&|| export.pending.borrow().is_empty());
    assert!(application.windows().is_empty());
    drop(response);
    drop(export);
    let removed = invoke(&intruder, PATH, INTERFACE, "OpenFile", arguments("removed"));
    until(&|| removed.borrow().is_some());
    assert!(removed.borrow_mut().take().unwrap().is_err());
    server.close_sync(None::<&gio::Cancellable>).unwrap();
    intruder.close_sync(None::<&gio::Cancellable>).unwrap();
}
