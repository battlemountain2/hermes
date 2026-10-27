use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

use gtk::{gio, glib, prelude::*};

pub(crate) mod request;
use request::{Options, Request};

const BUS: &str = "org.freedesktop.impl.portal.desktop.hermes";
const FRONTEND: &str = "org.freedesktop.portal.Desktop";
const INTERFACE: &str = "org.freedesktop.impl.portal.FileChooser";
const PATH: &str = "/org/freedesktop/portal/desktop";
const XML: &str = include_str!("portal/interface.xml");

struct Pending {
    sender: String,
    invocation: gio::DBusMethodInvocation,
    registration: gio::RegistrationId,
    picker: Option<crate::ui::picker::Picker>,
}

struct Export {
    connection: gio::DBusConnection,
    application: glib::WeakRef<gtk::Application>,
    pending: RefCell<BTreeMap<String, Pending>>,
    registration: RefCell<Option<gio::RegistrationId>>,
    subscription: RefCell<Option<gio::SignalSubscription>>,
}

impl Export {
    fn new(
        connection: &gio::DBusConnection,
        application: &gtk::Application,
    ) -> Result<Rc<Self>, glib::Error> {
        let node = gio::DBusNodeInfo::for_xml(XML)?;
        let interface = node.lookup_interface(INTERFACE).ok_or_else(|| {
            glib::Error::new(gio::IOErrorEnum::Failed, "Missing chooser interface")
        })?;
        let export = Rc::new(Self {
            connection: connection.clone(),
            application: application.downgrade(),
            pending: RefCell::new(BTreeMap::new()),
            registration: RefCell::new(None),
            subscription: RefCell::new(None),
        });
        let weak = Rc::downgrade(&export);
        let registration = connection
            .register_object(PATH, &interface)
            .property(|_, _, _, _, property| {
                if property == "version" {
                    4_u32.to_variant()
                } else {
                    0_u32.to_variant()
                }
            })
            .method_call(move |_, sender, _, _, method, parameters, invocation| {
                let Some(sender) = sender else {
                    invocation.return_error(gio::DBusError::AccessDenied, "Missing portal caller");
                    return;
                };
                if let Some(export) = weak.upgrade() {
                    export.begin(sender, method, parameters, invocation);
                } else {
                    invocation.return_error(gio::DBusError::Failed, "Chooser is closed");
                }
            })
            .build()?;
        export.registration.replace(Some(registration));
        let weak = Rc::downgrade(&export);
        let subscription = connection.subscribe_to_signal(
            Some("org.freedesktop.DBus"),
            Some("org.freedesktop.DBus"),
            Some("NameOwnerChanged"),
            Some("/org/freedesktop/DBus"),
            None,
            gio::DBusSignalFlags::NONE,
            move |signal| {
                let Some((name, _, owner)) = signal.parameters.get::<(String, String, String)>()
                else {
                    return;
                };
                if !owner.is_empty() {
                    return;
                }
                if let Some(export) = weak.upgrade() {
                    let paths = export
                        .pending
                        .borrow()
                        .iter()
                        .filter(|(_, pending)| pending.sender == name)
                        .map(|(path, _)| path.clone())
                        .collect::<Vec<_>>();
                    for path in paths {
                        export.finish(&path, 1, Options::new());
                    }
                }
            },
        );
        export.subscription.replace(Some(subscription));
        Ok(export)
    }

    fn begin(
        self: &Rc<Self>,
        sender: &str,
        method: &str,
        parameters: glib::Variant,
        invocation: gio::DBusMethodInvocation,
    ) {
        if parameters.size() > 64 * 1024 || self.pending.borrow().len() >= 8 {
            invocation.return_error(
                gio::DBusError::LimitsExceeded,
                "Too many or oversized chooser requests",
            );
            return;
        }
        let Some((handle, app_id, parent, title, options)) =
            parameters.get::<(glib::variant::ObjectPath, String, String, String, Options)>()
        else {
            invocation.return_error(gio::DBusError::InvalidArgs, "Invalid chooser parameters");
            return;
        };
        let handle = handle.as_str().to_owned();
        if !handle.starts_with("/org/freedesktop/portal/desktop/request/")
            || handle.len() > 4096
            || app_id.len() > 1024
            || self.pending.borrow().contains_key(&handle)
        {
            invocation.return_error(
                gio::DBusError::InvalidArgs,
                "Invalid or duplicate chooser handle",
            );
            return;
        }
        let request = match Request::parse(method, title, parent, options) {
            Ok(request) => request,
            Err(message) => {
                invocation.return_error(gio::DBusError::InvalidArgs, &message);
                return;
            }
        };
        let node = gio::DBusNodeInfo::for_xml(XML).expect("Compiled chooser XML");
        let interface = node
            .lookup_interface("org.freedesktop.impl.portal.Request")
            .expect("Compiled request interface");
        let weak = Rc::downgrade(self);
        let close_path = handle.clone();
        let registration = match self
            .connection
            .register_object(&handle, &interface)
            .method_call(move |_, sender, _, _, _, _, invocation| {
                let Some(export) = weak.upgrade() else {
                    invocation.return_value(None);
                    return;
                };
                let authorized = export
                    .pending
                    .borrow()
                    .get(&close_path)
                    .is_some_and(|pending| Some(pending.sender.as_str()) == sender);
                if !authorized {
                    invocation.return_error(
                        gio::DBusError::AccessDenied,
                        "Request belongs to another caller",
                    );
                    return;
                }
                invocation.return_value(None);
                export.finish(&close_path, 1, Options::new());
            })
            .build()
        {
            Ok(registration) => registration,
            Err(error) => {
                invocation.return_error(gio::DBusError::Failed, &error.to_string());
                return;
            }
        };
        self.pending.borrow_mut().insert(
            handle.clone(),
            Pending {
                sender: sender.to_owned(),
                invocation,
                registration,
                picker: None,
            },
        );
        let weak = Rc::downgrade(self);
        let connection = self.connection.clone();
        let sender = sender.to_owned();
        glib::MainContext::default().spawn_local(async move {
            let reply = connection
                .call_future(
                    Some("org.freedesktop.DBus"),
                    "/org/freedesktop/DBus",
                    "org.freedesktop.DBus",
                    "GetNameOwner",
                    Some(&(FRONTEND,).to_variant()),
                    None,
                    gio::DBusCallFlags::NONE,
                    3000,
                )
                .await;
            let authorized = reply
                .ok()
                .and_then(|reply| reply.get::<(String,)>())
                .is_some_and(|(owner,)| owner == sender);
            let Some(export) = weak.upgrade() else {
                return;
            };
            if !export.pending.borrow().contains_key(&handle) {
                return;
            }
            if !authorized {
                export.finish(&handle, 2, Options::new());
                return;
            }
            let Some(application) = export.application.upgrade() else {
                export.finish(&handle, 2, Options::new());
                return;
            };
            let done = Rc::downgrade(&export);
            let done_path = handle.clone();
            let picker = crate::ui::picker::Picker::new(
                &application,
                request,
                Rc::new(move |results| {
                    if let Some(export) = done.upgrade() {
                        match results {
                            Some(results) => export.finish(&done_path, 0, results),
                            None => export.finish(&done_path, 1, Options::new()),
                        }
                    }
                }),
            );
            export
                .pending
                .borrow_mut()
                .get_mut(&handle)
                .expect("Current chooser request")
                .picker = Some(picker);
        });
    }

    fn finish(&self, path: &str, response: u32, results: Options) {
        let pending = self.pending.borrow_mut().remove(path);
        if let Some(pending) = pending {
            let _removed = self.connection.unregister_object(pending.registration);
            pending
                .invocation
                .return_value(Some(&(response, results).to_variant()));
            if let Some(picker) = pending.picker {
                picker.close();
            }
        }
    }

    fn cancel_all(&self) {
        let paths = self.pending.borrow().keys().cloned().collect::<Vec<_>>();
        for path in paths {
            self.finish(&path, 1, Options::new());
        }
    }
}

impl Drop for Export {
    fn drop(&mut self) {
        self.cancel_all();
        self.subscription.borrow_mut().take();
        if let Some(registration) = self.registration.borrow_mut().take() {
            let _removed = self.connection.unregister_object(registration);
        }
    }
}

pub(crate) fn run() -> glib::ExitCode {
    let marker = crate::paths::picker_integration_file();
    if !crate::file_manager::enabled_at(&marker) {
        eprintln!("Hermes Open/Save picker is not enabled");
        return glib::ExitCode::FAILURE;
    }
    glib::set_application_name(crate::identity::NAME);
    let application = gtk::Application::builder()
        .application_id("io.github.battlemountain2.Hermes.FileChooser")
        .flags(gio::ApplicationFlags::IS_SERVICE)
        .inactivity_timeout(30_000)
        .build();
    application.connect_startup(move |application| {
        if let Err(error) = crate::assets::prepare() {
            eprintln!("Unable to prepare picker icons: {error}");
        }
        crate::assets::register_icon_theme();
        gtk::Window::set_default_icon_name(crate::identity::ICON_NAME);
        let theme = crate::ui::picker::prepare();
        let Some(connection) = application.dbus_connection() else {
            application.quit();
            return;
        };
        let exported = match Export::new(&connection, application) {
            Ok(exported) => exported,
            Err(error) => {
                eprintln!("Unable to register chooser: {error}");
                application.quit();
                return;
            }
        };
        let lost_app = application.downgrade();
        let owner = gio::bus_own_name_on_connection(
            &connection,
            BUS,
            gio::BusNameOwnerFlags::DO_NOT_QUEUE,
            |_, _| {},
            move |_, _| {
                if let Some(application) = lost_app.upgrade() {
                    application.quit();
                }
            },
        );
        let monitor = gio::File::for_path(&marker)
            .monitor_file(
                gio::FileMonitorFlags::WATCH_MOVES,
                None::<&gio::Cancellable>,
            )
            .ok();
        if let Some(monitor) = monitor.as_ref() {
            let weak_app = application.downgrade();
            let marker = marker.clone();
            monitor.connect_changed(move |_, _, _, _| {
                if !crate::file_manager::enabled_at(&marker)
                    && let Some(application) = weak_app.upgrade()
                {
                    application.quit();
                }
            });
        }
        let owner = RefCell::new(Some(owner));
        application.connect_shutdown(move |_| {
            exported.cancel_all();
            if let Some(owner) = owner.borrow_mut().take() {
                gio::bus_unown_name(owner);
            }
            if let Some(monitor) = monitor.as_ref() {
                monitor.cancel();
            }
            let _retained_theme = &theme;
        });
    });
    application.run_with_args(&["hermes-chooser"])
}

#[cfg(test)]
mod tests;
