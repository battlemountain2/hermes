use std::{cell::RefCell, fs, io::Read, os::unix::fs::OpenOptionsExt, path::Path, rc::Rc};

use gtk::{gio, glib, prelude::*};

use crate::model::Location;

const BUS: &str = "org.freedesktop.FileManager1";
const PATH: &str = "/org/freedesktop/FileManager1";
const XML: &str = include_str!("file_manager/interface.xml");
const MAX_URIS: usize = 64;
const MAX_WINDOWS: usize = 16;
const NO_FOLLOW: i32 = 0x20000;
const NON_BLOCK: i32 = 0x800;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Request {
    pub directory: Location,
    pub selection: Vec<Location>,
    pub properties: bool,
    pub startup_id: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Preference {
    version: u32,
    enabled: bool,
}

pub(crate) fn enabled_at(path: &Path) -> bool {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    if !metadata.is_file() || metadata.len() > 1024 {
        return false;
    }
    let Ok(file) = fs::OpenOptions::new()
        .read(true)
        .custom_flags(NO_FOLLOW | NON_BLOCK)
        .open(path)
    else {
        return false;
    };
    if !file
        .metadata()
        .is_ok_and(|metadata| metadata.is_file() && metadata.len() <= 1024)
    {
        return false;
    }
    let mut bytes = Vec::new();
    file.take(1025).read_to_end(&mut bytes).is_ok()
        && bytes.len() <= 1024
        && serde_json::from_slice::<Preference>(&bytes)
            .is_ok_and(|preference| preference.version == 1 && preference.enabled)
}

fn location(file: &gio::File) -> Location {
    file.path()
        .map(Location::local)
        .unwrap_or_else(|| Location::uri(file.uri()))
}

fn requests(method: &str, uris: &[String], startup_id: &str) -> Result<Vec<Request>, &'static str> {
    let properties = match method {
        "ShowFolders" | "ShowItems" => false,
        "ShowItemProperties" => true,
        _ => return Err("Unknown file-manager method"),
    };
    if uris.is_empty()
        || uris.len() > MAX_URIS
        || startup_id.len() > 1024
        || startup_id.chars().any(char::is_control)
    {
        return Err("Expected between 1 and 64 bounded file-manager locations");
    }
    let mut result: Vec<Request> = Vec::new();
    let mut bytes = 0_usize;
    for uri in uris {
        bytes = bytes.saturating_add(uri.len());
        if uri.len() > 16 * 1024 || bytes > 64 * 1024 || uri.chars().any(char::is_control) {
            return Err("File-manager locations exceed the request limit");
        }
        let parsed = glib::Uri::parse(uri, glib::UriFlags::ENCODED)
            .map_err(|_| "Expected an absolute file or supported filesystem URI")?;
        let scheme = parsed.scheme();
        if !matches!(
            scheme.as_str(),
            "file"
                | "smb"
                | "sftp"
                | "ftp"
                | "ftps"
                | "dav"
                | "davs"
                | "afp"
                | "mtp"
                | "gphoto2"
                | "trash"
                | "network"
                | "computer"
        ) || parsed.query().is_some()
            || parsed.fragment().is_some()
            || (scheme == "file"
                && parsed
                    .host()
                    .is_some_and(|host| !host.is_empty() && host != "localhost"))
        {
            return Err("Unsupported file-manager URI");
        }
        let file = gio::File::for_uri(uri);
        if scheme == "file" && !file.path().is_some_and(|path| path.is_absolute()) {
            return Err("Expected an absolute local file URI");
        }
        let target = location(&file);
        let parent = (method != "ShowFolders").then(|| file.parent()).flatten();
        let directory = parent
            .as_ref()
            .map(location)
            .unwrap_or_else(|| target.clone());
        let selected = parent.is_some().then_some(target);
        if !properties
            && let Some(existing) = result
                .iter_mut()
                .find(|request| request.directory == directory)
        {
            if let Some(selected) = selected
                && !existing.selection.contains(&selected)
            {
                existing.selection.push(selected);
            }
            continue;
        }
        if result.len() >= MAX_WINDOWS {
            return Err("Open at most 16 locations per file-manager request");
        }
        result.push(Request {
            directory,
            selection: selected.into_iter().collect(),
            properties,
            startup_id: startup_id.to_owned(),
        });
    }
    Ok(result)
}

struct Export {
    connection: gio::DBusConnection,
    registration: Option<gio::RegistrationId>,
}

impl Export {
    fn new(
        connection: &gio::DBusConnection,
        handler: Rc<dyn Fn(Request)>,
    ) -> Result<Self, glib::Error> {
        let node = gio::DBusNodeInfo::for_xml(XML)?;
        let interface = node.lookup_interface(BUS).ok_or_else(|| {
            glib::Error::new(gio::IOErrorEnum::Failed, "Missing FileManager1 interface")
        })?;
        let registration = connection
            .register_object(PATH, &interface)
            .method_call(move |_, _, _, _, method, parameters, invocation| {
                let Some((uris, startup_id)) = parameters.get::<(Vec<String>, String)>() else {
                    invocation.return_error(gio::DBusError::InvalidArgs, "Expected (as, s)");
                    return;
                };
                match requests(method, &uris, &startup_id) {
                    Ok(requests) => {
                        for request in requests {
                            handler(request);
                        }
                        invocation.return_value(None);
                    }
                    Err(message) => invocation.return_error(gio::DBusError::InvalidArgs, message),
                }
            })
            .build()?;
        Ok(Self {
            connection: connection.clone(),
            registration: Some(registration),
        })
    }
}

impl Drop for Export {
    fn drop(&mut self) {
        if let Some(registration) = self.registration.take() {
            let _removed = self.connection.unregister_object(registration);
        }
    }
}

struct Owner {
    id: Option<gio::OwnerId>,
    export: Rc<RefCell<Option<Export>>>,
}

impl Owner {
    fn start(connection: &gio::DBusConnection, handler: Rc<dyn Fn(Request)>) -> Self {
        let export = Rc::new(RefCell::new(None));
        let bus_slot = Rc::downgrade(&export);
        let lost_slot = Rc::downgrade(&export);
        let id = gio::bus_own_name_on_connection(
            connection,
            BUS,
            gio::BusNameOwnerFlags::DO_NOT_QUEUE,
            move |connection, _| {
                let Some(slot) = bus_slot.upgrade() else {
                    return;
                };
                match Export::new(&connection, handler.clone()) {
                    Ok(export) => {
                        slot.replace(Some(export));
                    }
                    Err(error) => {
                        tracing::warn!(%error, "Hermes folder-reveal service unavailable")
                    }
                }
            },
            move |_, _| {
                if let Some(slot) = lost_slot.upgrade() {
                    slot.borrow_mut().take();
                }
                tracing::info!("Folder-reveal name unavailable; existing file manager kept");
            },
        );
        Self {
            id: Some(id),
            export,
        }
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        if let Some(id) = self.id.take() {
            gio::bus_unown_name(id);
        }
        self.export.borrow_mut().take();
    }
}

pub(crate) fn initialize(application: &gtk::Application) {
    if application.application_id().as_deref() != Some(crate::identity::APPLICATION_ID) {
        return;
    }
    let path = crate::paths::folder_integration_file();
    let Some(connection) = application.dbus_connection() else {
        return;
    };
    let enabled = enabled_at(&path);
    if application
        .flags()
        .contains(gio::ApplicationFlags::IS_SERVICE)
    {
        if !enabled {
            application.quit();
            return;
        }
        application.set_inactivity_timeout(30_000);
    }
    let weak_app = application.downgrade();
    let handler: Rc<dyn Fn(Request)> = Rc::new(move |request| {
        if let Some(application) = weak_app.upgrade() {
            crate::ui::present_reveal(&application, request);
        }
    });
    let owner = Rc::new(RefCell::new(
        enabled.then(|| Owner::start(&connection, handler.clone())),
    ));
    let monitor = gio::File::for_path(&path)
        .monitor_file(
            gio::FileMonitorFlags::WATCH_MOVES,
            None::<&gio::Cancellable>,
        )
        .ok();
    if let Some(monitor) = monitor.as_ref() {
        let watched_owner = Rc::downgrade(&owner);
        monitor.connect_changed(move |_, _, _, _| {
            let Some(owner) = watched_owner.upgrade() else {
                return;
            };
            if enabled_at(&path) {
                if owner.borrow().is_none() {
                    owner.replace(Some(Owner::start(&connection, handler.clone())));
                }
            } else {
                owner.borrow_mut().take();
            }
        });
    }
    application.connect_shutdown(move |_| {
        if let Some(monitor) = monitor.as_ref() {
            monitor.cancel();
        }
        owner.borrow_mut().take();
    });
}

#[cfg(test)]
mod tests;
