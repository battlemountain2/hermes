use std::{cell::RefCell, collections::BTreeMap, path::PathBuf, rc::Rc, time::Duration};

use gtk::{gio, glib, prelude::*};

use crate::music::{Repeat, Track};

const PATH: &str = "/org/mpris/MediaPlayer2";
const ROOT: &str = "org.mpris.MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";
const NO_TRACK: &str = "/org/mpris/MediaPlayer2/TrackList/NoTrack";

mod art;

#[derive(Clone)]
pub(crate) struct Snapshot {
    pub track: Option<Track>,
    pub track_id: String,
    pub status: &'static str,
    pub position: i64,
    pub duration: i64,
    pub volume: f64,
    pub repeat: Repeat,
    pub shuffle: bool,
    pub can_next: bool,
    pub can_previous: bool,
    pub can_play: bool,
    pub can_pause: bool,
    pub can_seek: bool,
    pub seek_revision: u64,
    pub art_url: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Command {
    Play,
    Pause,
    PlayPause,
    Stop,
    Next,
    Previous,
    Seek(i64),
    SetPosition(String, i64),
    SetVolume(f64),
    SetRepeat(Repeat),
    SetShuffle(bool),
    OpenUri(PathBuf),
}

pub(crate) trait Backend {
    fn snapshot(&self) -> Snapshot;
    fn command(&self, command: Command) -> Result<(), String>;
}

pub(crate) struct Service {
    owner: Option<gio::OwnerId>,
    exported: Rc<RefCell<Option<Rc<Export>>>>,
}

impl Service {
    pub fn start(application: &gtk::Application, backend: Rc<dyn Backend>) -> Self {
        let exported = Rc::new(RefCell::new(None));
        let slot = Rc::downgrade(&exported);
        let weak_app = application.downgrade();
        let raise_app = weak_app.clone();
        let raise: Rc<dyn Fn()> = Rc::new(move || {
            if let Some(application) = raise_app.upgrade() {
                if let Some(window) = application.active_window() {
                    window.present();
                } else {
                    application.activate();
                }
            }
        });
        let quit: Rc<dyn Fn()> = Rc::new(move || {
            if let Some(application) = weak_app.upgrade() {
                application.quit();
            }
        });
        let name = if application
            .application_id()
            .is_some_and(|id| id.ends_with(".Dev"))
        {
            "org.mpris.MediaPlayer2.hermes.Dev"
        } else {
            "org.mpris.MediaPlayer2.hermes"
        };
        let acquired_slot = slot.clone();
        let lost_slot = slot.clone();
        let owner = gio::bus_own_name(
            gio::BusType::Session,
            name,
            gio::BusNameOwnerFlags::DO_NOT_QUEUE,
            move |connection, _| {
                let Some(slot) = slot.upgrade() else {
                    return;
                };
                match Export::new(
                    &connection,
                    backend.clone(),
                    raise.clone(),
                    quit.clone(),
                    true,
                ) {
                    Ok(export) => {
                        export.start_updates();
                        slot.replace(Some(export));
                    }
                    Err(error) => tracing::warn!(%error, "Desktop playback controls unavailable"),
                }
            },
            move |_, name| {
                if acquired_slot.upgrade().is_some() {
                    tracing::debug!(name, "Desktop playback controls registered");
                }
            },
            move |_, name| {
                if let Some(slot) = lost_slot.upgrade() {
                    slot.borrow_mut().take();
                }
                tracing::warn!(name, "Desktop playback bus name unavailable");
            },
        );
        Self {
            owner: Some(owner),
            exported,
        }
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        if let Some(owner) = self.owner.take() {
            gio::bus_unown_name(owner);
        }
        self.exported.borrow_mut().take();
    }
}

struct Export {
    connection: gio::DBusConnection,
    registrations: RefCell<Vec<gio::RegistrationId>>,
    backend: Rc<dyn Backend>,
    previous: RefCell<BTreeMap<String, glib::Variant>>,
    seek_revision: std::cell::Cell<u64>,
    timer: RefCell<Option<glib::SourceId>>,
    artwork: Option<art::Artwork>,
}

impl Export {
    fn new(
        connection: &gio::DBusConnection,
        backend: Rc<dyn Backend>,
        raise: Rc<dyn Fn()>,
        quit: Rc<dyn Fn()>,
        artwork: bool,
    ) -> Result<Rc<Self>, glib::Error> {
        let node = gio::DBusNodeInfo::for_xml(include_str!("mpris/interfaces.xml"))?;
        let export = Rc::new(Self {
            connection: connection.clone(),
            registrations: RefCell::new(Vec::new()),
            backend,
            previous: RefCell::new(BTreeMap::new()),
            seek_revision: std::cell::Cell::new(0),
            timer: RefCell::new(None),
            artwork: artwork.then(art::Artwork::new),
        });
        for interface in node.interfaces() {
            let weak = Rc::downgrade(&export);
            let get_weak = weak.clone();
            let set_weak = weak.clone();
            let raise = raise.clone();
            let quit = quit.clone();
            let registration = connection
                .register_object(PATH, interface)
                .method_call(move |_, _, _, interface, method, parameters, invocation| {
                    let Some(export) = weak.upgrade() else {
                        invocation.return_dbus_error(
                            "org.freedesktop.DBus.Error.Failed",
                            "Player closed",
                        );
                        return;
                    };
                    let result = if interface == Some(ROOT) {
                        match method {
                            "Raise" => {
                                raise();
                                Ok(())
                            }
                            "Quit" => {
                                quit();
                                Ok(())
                            }
                            _ => Err("Unknown player method".into()),
                        }
                    } else {
                        parse_method(method, &parameters)
                            .and_then(|command| export.backend.command(command))
                    };
                    match result {
                        Ok(()) => invocation.return_value(Some(&().to_variant())),
                        Err(error) => invocation
                            .return_dbus_error("org.freedesktop.DBus.Error.InvalidArgs", &error),
                    }
                    export.publish();
                })
                .property(move |_, _, _, interface, property| {
                    get_weak
                        .upgrade()
                        .and_then(|export| export.properties(interface).remove(property))
                        .unwrap_or_else(|| ().to_variant())
                })
                .set_property(move |_, _, _, _, property, value| {
                    let Some(export) = set_weak.upgrade() else {
                        return false;
                    };
                    let accepted = property == "Rate" && value.get::<f64>() == Some(1.0)
                        || parse_property(property, &value)
                            .is_ok_and(|command| export.backend.command(command).is_ok());
                    if accepted {
                        export.publish();
                    }
                    accepted
                })
                .build()?;
            export.registrations.borrow_mut().push(registration);
        }
        let snapshot = export.backend.snapshot();
        export.seek_revision.set(snapshot.seek_revision);
        export.previous.replace(player_properties(&snapshot));
        Ok(export)
    }

    fn properties(&self, interface: &str) -> BTreeMap<String, glib::Variant> {
        if interface == ROOT {
            root_properties()
        } else {
            let mut snapshot = self.backend.snapshot();
            if let Some(artwork) = &self.artwork {
                snapshot.art_url = snapshot.art_url.or_else(|| artwork.url(&snapshot.track));
            }
            let mut properties = player_properties(&snapshot);
            properties.insert("Position".into(), snapshot.position.max(0).to_variant());
            properties
        }
    }

    fn start_updates(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.timer.replace(Some(glib::timeout_add_local(
            Duration::from_millis(250),
            move || {
                let Some(export) = weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                export.publish();
                glib::ControlFlow::Continue
            },
        )));
    }

    fn publish(&self) {
        let mut snapshot = self.backend.snapshot();
        if let Some(artwork) = &self.artwork {
            artwork.update(if snapshot.status == "Stopped" {
                None
            } else {
                snapshot.track.as_ref()
            });
            snapshot.art_url = snapshot.art_url.or_else(|| artwork.url(&snapshot.track));
        }
        let properties = player_properties(&snapshot);
        let previous = self.previous.replace(properties.clone());
        let changed: BTreeMap<_, _> = properties
            .into_iter()
            .filter(|(key, value)| previous.get(key) != Some(value))
            .collect();
        if !changed.is_empty() {
            self.emit(
                "org.freedesktop.DBus.Properties",
                "PropertiesChanged",
                &(PLAYER, changed, Vec::<String>::new()).to_variant(),
            );
        }
        if self.seek_revision.replace(snapshot.seek_revision) != snapshot.seek_revision {
            self.emit(PLAYER, "Seeked", &(snapshot.position.max(0),).to_variant());
        }
    }

    fn emit(&self, interface: &str, name: &str, parameters: &glib::Variant) {
        if let Err(error) =
            self.connection
                .emit_signal(None, PATH, interface, name, Some(parameters))
        {
            tracing::debug!(%error, name, "Desktop playback update failed");
        }
    }
}

impl Drop for Export {
    fn drop(&mut self) {
        if let Some(timer) = self.timer.get_mut().take() {
            timer.remove();
        }
        for registration in self.registrations.get_mut().drain(..) {
            let _result = self.connection.unregister_object(registration);
        }
    }
}

fn root_properties() -> BTreeMap<String, glib::Variant> {
    BTreeMap::from([
        ("CanQuit".into(), true.to_variant()),
        ("CanRaise".into(), true.to_variant()),
        ("HasTrackList".into(), false.to_variant()),
        ("Identity".into(), "Hermes".to_variant()),
        ("SupportedUriSchemes".into(), vec!["file"].to_variant()),
        (
            "SupportedMimeTypes".into(),
            vec![
                "audio/flac",
                "audio/x-flac",
                "audio/mpeg",
                "audio/ogg",
                "audio/opus",
                "audio/mp4",
                "audio/aac",
                "audio/x-wav",
                "audio/wav",
                "audio/x-ms-wma",
            ]
            .to_variant(),
        ),
    ])
}

fn player_properties(snapshot: &Snapshot) -> BTreeMap<String, glib::Variant> {
    let mut metadata = BTreeMap::<String, glib::Variant>::new();
    if let Some(track) = &snapshot.track {
        let track_id = glib::variant::ObjectPath::try_from(snapshot.track_id.clone())
            .unwrap_or_else(|_| {
                glib::variant::ObjectPath::try_from(NO_TRACK).expect("Constant object path")
            });
        metadata.insert("mpris:trackid".into(), track_id.to_variant());
        metadata.insert("xesam:title".into(), track.title().to_variant());
        metadata.insert("xesam:artist".into(), vec![track.artist()].to_variant());
        metadata.insert("xesam:album".into(), track.album().to_variant());
        metadata.insert(
            "xesam:url".into(),
            gio::File::for_path(&track.path).uri().to_variant(),
        );
        if let Some(number) = track.tags.track {
            metadata.insert(
                "xesam:trackNumber".into(),
                (number.min(i32::MAX as u32) as i32).to_variant(),
            );
        }
        if snapshot.duration > 0 {
            metadata.insert("mpris:length".into(), snapshot.duration.to_variant());
        }
        if let Some(url) = &snapshot.art_url {
            metadata.insert("mpris:artUrl".into(), url.to_variant());
        }
    }
    BTreeMap::from([
        ("PlaybackStatus".into(), snapshot.status.to_variant()),
        (
            "LoopStatus".into(),
            match snapshot.repeat {
                Repeat::Off => "None",
                Repeat::All => "Playlist",
                Repeat::One => "Track",
            }
            .to_variant(),
        ),
        ("Rate".into(), 1_f64.to_variant()),
        ("MinimumRate".into(), 1_f64.to_variant()),
        ("MaximumRate".into(), 1_f64.to_variant()),
        ("Shuffle".into(), snapshot.shuffle.to_variant()),
        ("Volume".into(), snapshot.volume.to_variant()),
        ("Metadata".into(), metadata.to_variant()),
        ("CanGoNext".into(), snapshot.can_next.to_variant()),
        ("CanGoPrevious".into(), snapshot.can_previous.to_variant()),
        ("CanPlay".into(), snapshot.can_play.to_variant()),
        ("CanPause".into(), snapshot.can_pause.to_variant()),
        ("CanSeek".into(), snapshot.can_seek.to_variant()),
        ("CanControl".into(), true.to_variant()),
    ])
}

fn parse_method(method: &str, parameters: &glib::Variant) -> Result<Command, String> {
    match method {
        "Play" | "Pause" | "PlayPause" | "Stop" | "Next" | "Previous"
            if parameters.get::<()>().is_some() =>
        {
            Some(match method {
                "Play" => Command::Play,
                "Pause" => Command::Pause,
                "PlayPause" => Command::PlayPause,
                "Stop" => Command::Stop,
                "Next" => Command::Next,
                _ => Command::Previous,
            })
        }
        "Seek" => parameters
            .get::<(i64,)>()
            .map(|(offset,)| Command::Seek(offset)),
        "SetPosition" => parameters
            .get::<(glib::variant::ObjectPath, i64)>()
            .map(|(track, position)| Command::SetPosition(track.to_string(), position)),
        "OpenUri" => parameters
            .get::<(String,)>()
            .and_then(|(uri,)| local_audio_uri(&uri).map(Command::OpenUri)),
        _ => None,
    }
    .ok_or_else(|| {
        "Unsupported method or invalid arguments; only local audio files can be opened".into()
    })
}

fn local_audio_uri(uri: &str) -> Option<PathBuf> {
    if uri.len() > 16_384 || !(uri.starts_with("file:///") || uri.starts_with("file://localhost/"))
    {
        return None;
    }
    let path = gio::File::for_uri(uri).path()?;
    (path.is_absolute()
        && crate::services::classify_by_name(path.as_os_str())
            == crate::services::FormatFamily::Audio)
        .then_some(path)
}

fn parse_property(property: &str, value: &glib::Variant) -> Result<Command, String> {
    match property {
        "Volume" => value
            .get::<f64>()
            .filter(|value| value.is_finite())
            .map(|value| Command::SetVolume(value.clamp(0.0, 1.0))),
        "Shuffle" => value.get::<bool>().map(Command::SetShuffle),
        "LoopStatus" => value
            .str()
            .and_then(|value| match value {
                "None" => Some(Repeat::Off),
                "Playlist" => Some(Repeat::All),
                "Track" => Some(Repeat::One),
                _ => None,
            })
            .map(Command::SetRepeat),
        "Rate" if value.get::<f64>() == Some(0.0) => Some(Command::Pause),
        _ => None,
    }
    .ok_or_else(|| "Unsupported property value".into())
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod playback_tests;
