use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    time::Duration,
};

use gtk::{glib, prelude::*};

use super::preview::stream::PlaybackStream;
use crate::{
    assets::{self, icons},
    model::{EntryKind, FileEntry, FileEntryInner, Location, MetadataValue},
    music::{Queue, Repeat, Track},
    sandbox::Cancellation,
};

mod collections;
mod context_menu;
pub(crate) mod controller;
mod queue;
mod session;

use crate::services::music_collections::{Command as CollectionCommand, Store as CollectionStore};
use collections::{Group, Section};
use controller::Controller;

#[derive(Clone)]
pub(crate) struct MusicPlayer {
    state: Rc<State>,
}

struct State {
    controller: Rc<Controller>,
    displayed_revision: Cell<u64>,
    displayed_track: RefCell<Option<Track>>,
    playing_icon: Cell<bool>,
    muted_icon: Cell<bool>,
    repeat_icon: Cell<Repeat>,
    speaker: gtk::Button,
    bar: gtk::Box,
    view: gtk::Box,
    title: gtk::Label,
    detail: gtk::Label,
    clock: gtk::Label,
    status: gtk::Label,
    cover: gtk::Image,
    cover_button: gtk::Button,
    play: gtk::Button,
    #[cfg(test)]
    seek: gtk::Scale,
    seek_control: Rc<super::seek::SeekControl>,
    volume: gtk::Scale,
    syncing: Cell<bool>,
    waveform: gtk::DrawingArea,
    waveform_status: gtk::Label,
    waveform_toggle: gtk::ToggleButton,
    disposed: Cell<bool>,
    scan: RefCell<Option<crate::music::library::Scan>>,
    library: RefCell<Vec<Track>>,
    library_revision: Cell<u64>,
    albums: RefCell<Vec<Album>>,
    album_model: gtk::StringList,
    album_selection: gtk::SingleSelection,
    rendering_library: Cell<bool>,
    selected_album: RefCell<Option<PathBuf>>,
    tracks: RefCell<Vec<Track>>,
    track_model: gtk::StringList,
    track_selection: gtk::SingleSelection,
    queue_model: gtk::StringList,
    queue_indices: RefCell<Vec<usize>>,
    queue_button: gtk::MenuButton,
    queue_panel: gtk::Box,
    queue_toggle: gtk::ToggleButton,
    queue_summary: gtk::Label,
    queue_empty: gtk::Label,
    queue_resume: gtk::Button,
    queue_clear: gtk::Button,
    shuffle: gtk::ToggleButton,
    repeat: gtk::Button,
    filter: gtk::Entry,
    play_album: gtk::Button,
    selected_actions: Vec<gtk::Button>,
    selection_options: gtk::MenuButton,
    collection_store: CollectionStore,
    collection_snapshot: RefCell<std::sync::Arc<crate::services::music_collections::Snapshot>>,
    section: Cell<Section>,
    navigation: Vec<(Section, gtk::ToggleButton)>,
    album_scroll: gtk::ScrolledWindow,
    group_scroll: gtk::ScrolledWindow,
    group_model: gtk::StringList,
    group_labels: RefCell<Vec<String>>,
    group_section: Cell<Option<Section>>,
    group_library_revision: Cell<u64>,
    group_selection: gtk::SingleSelection,
    groups: RefCell<Vec<Group>>,
    playlist_editor: gtk::Box,
    collection_status: gtk::Label,
    favorite_buttons: RefCell<Vec<(glib::WeakRef<gtk::ToggleButton>, PathBuf)>>,
    favorite_paths: RefCell<std::collections::BTreeSet<PathBuf>>,
    catalog_paths: RefCell<std::collections::BTreeSet<PathBuf>>,
    pending_observation: RefCell<Option<collections::PendingObservation>>,
}

#[derive(Clone, Eq, PartialEq)]
struct Album {
    path: PathBuf,
    title: String,
    artist: String,
    count: usize,
}

impl std::ops::Deref for State {
    type Target = Controller;

    fn deref(&self) -> &Self::Target {
        &self.controller
    }
}

impl MusicPlayer {
    pub(crate) fn for_application(application: &gtk::Application) -> Self {
        Self::with_controller(Controller::for_application(application))
    }

    pub(crate) fn controller(&self) -> Rc<Controller> {
        self.state.controller.clone()
    }

    #[cfg(test)]
    fn with_session(session_path: Option<PathBuf>) -> Self {
        let controller = Controller::with_session(session_path);
        let store = controller.collection_store.borrow().clone();
        Self::with_collection_store(controller, store)
    }

    fn with_controller(controller: Rc<Controller>) -> Self {
        let store = controller.collection_store.borrow().clone();
        Self::with_collection_store(controller, store)
    }

    fn with_collection_store(
        controller: Rc<Controller>,
        collection_store: CollectionStore,
    ) -> Self {
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        bar.add_css_class("music-transport");
        bar.set_visible(false);
        let cover = assets::primary_icon(icons::MUSIC, 42);
        cover.set_size_request(48, 48);
        let cover_button = gtk::Button::builder()
            .child(&cover)
            .tooltip_text("Open the Music player · Ctrl+M")
            .build();
        cover_button.add_css_class("music-cover-button");
        bar.append(&cover_button);
        let labels = gtk::Box::new(gtk::Orientation::Vertical, 3);
        labels.set_size_request(180, -1);
        let title = text("Nothing playing", "music-title");
        title.set_max_width_chars(26);
        let detail = text("Choose a track in Music", "music-detail");
        detail.set_max_width_chars(26);
        labels.append(&title);
        labels.append(&detail);
        bar.append(&labels);
        let previous = button(
            icons::ARROW_LEFT,
            "Previous track; restart after three seconds",
        );
        let play = button(icons::PLAY, "Play / pause music");
        let next = button(icons::ARROW_RIGHT, "Next track");
        let stop = labeled_button(
            icons::X,
            "Stop playback",
            "Stop music and release the decoder",
        );
        let transport_controls = gtk::Box::new(gtk::Orientation::Horizontal, 4);
        transport_controls.add_css_class("music-transport-controls");
        for control in [&previous, &play, &next] {
            transport_controls.append(control);
        }
        bar.append(&transport_controls);
        let seek = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.1);
        seek.set_draw_value(false);
        seek.set_hexpand(true);
        seek.set_sensitive(false);
        seek.set_tooltip_text(Some("Seek through the complete track"));
        bar.append(&seek);
        let clock = text("0:00 / 0:00", "music-detail");
        bar.append(&clock);
        let speaker = button(icons::VOLUME, "Mute / unmute music");
        bar.append(&speaker);
        let volume = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 1.0, 0.05);
        volume.set_draw_value(false);
        volume.set_value(0.75);
        volume.set_size_request(80, -1);
        volume.set_tooltip_text(Some("Music volume; does not change system volume"));
        bar.append(&volume);

        let view = gtk::Box::new(gtk::Orientation::Vertical, 12);
        view.add_css_class("music-workspace");
        view.set_vexpand(true);
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        let heading = text("Music", "music-heading");
        let filter = gtk::Entry::builder()
            .placeholder_text("Find a song, artist, or album in your library…")
            .hexpand(true)
            .build();
        let refresh = gtk::Button::with_label("Refresh");
        refresh.add_css_class("music-control");
        refresh.set_tooltip_text(Some("Refresh the music library"));
        header.append(&heading);
        header.append(&filter);
        header.append(&refresh);
        let waveform_toggle = gtk::ToggleButton::builder().active(true).build();
        waveform_toggle.set_child(Some(&assets::primary_icon(icons::EYE, 16)));
        waveform_toggle.set_tooltip_text(Some("Hide the now-playing waveform"));
        waveform_toggle.update_property(&[gtk::accessible::Property::Label(
            "Show now-playing waveform",
        )]);
        waveform_toggle.add_css_class("music-control");
        waveform_toggle.add_css_class("music-panel-toggle");
        header.append(&waveform_toggle);
        let queue_toggle = gtk::ToggleButton::builder().active(true).build();
        queue_toggle.set_child(Some(&control_contents(icons::LIST, "Hide queue")));
        queue_toggle.set_tooltip_text(Some("Hide the queue panel; playback continues"));
        queue_toggle.update_property(&[gtk::accessible::Property::Label(
            "Show playback queue panel",
        )]);
        queue_toggle.add_css_class("music-control");
        queue_toggle.add_css_class("music-panel-toggle");
        header.append(&queue_toggle);
        view.append(&header);
        let navigation_row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        navigation_row.add_css_class("music-navigation");
        let mut navigation = Vec::new();
        for (section, label) in [
            (Section::Albums, "Albums"),
            (Section::Artists, "Artists"),
            (Section::Songs, "Songs"),
            (Section::Favorites, "Favorites"),
            (Section::Playlists, "Playlists"),
        ] {
            let control = gtk::ToggleButton::with_label(label);
            control.add_css_class("music-control");
            control.add_css_class("music-panel-toggle");
            if section == Section::Favorites {
                control.set_child(Some(&control_contents(icons::HEART, label)));
            }
            if let Some((_, first)) = navigation.first() {
                control.set_group(Some(first));
            }
            navigation_row.append(&control);
            navigation.push((section, control));
        }
        navigation[0].1.set_active(true);
        view.append(&navigation_row);
        let playlist_editor = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        playlist_editor.add_css_class("music-playlist-actions");
        let create_playlist = gtk::Button::with_label("Create playlist");
        let edit_playlist = gtk::Button::with_label("Edit playlist");
        for control in [&create_playlist, &edit_playlist] {
            control.add_css_class("music-control");
            playlist_editor.append(control);
        }
        view.append(&playlist_editor);
        playlist_editor.set_visible(false);
        let collection_status = text("", "music-detail");
        collection_status.set_wrap(true);
        collection_status.set_visible(false);
        view.append(&collection_status);
        let status = text(
            "Single-click to browse · double-click an album or song to play",
            "music-detail",
        );
        view.append(&status);
        let album_model = gtk::StringList::new(&[]);
        let album_selection = gtk::SingleSelection::new(Some(album_model.clone()));
        let albums = gtk::ListView::new(Some(album_selection.clone()), Some(label_factory()));
        albums.set_single_click_activate(false);
        albums.set_tooltip_text(Some(
            "Single-click to browse · double-click or Enter to play the full album",
        ));
        let album_scroll = scroller(&albums);
        album_scroll.set_size_request(245, -1);
        let track_model = gtk::StringList::new(&[]);
        let track_selection = gtk::SingleSelection::new(Some(track_model.clone()));
        let tracks = gtk::ListView::new(Some(track_selection.clone()), Some(label_factory()));
        tracks.set_single_click_activate(false);
        let track_scroll = scroller(&tracks);
        track_scroll.set_hexpand(true);
        let queue_model = gtk::StringList::new(&[]);
        let queue_view = gtk::ListView::new(
            Some(
                gtk::SingleSelection::builder()
                    .model(&queue_model)
                    .autoselect(false)
                    .can_unselect(true)
                    .build(),
            ),
            Some(label_factory()),
        );
        queue_view.set_single_click_activate(false);
        let popup_queue_view = gtk::ListView::new(
            Some(
                gtk::SingleSelection::builder()
                    .model(&queue_model)
                    .autoselect(false)
                    .can_unselect(true)
                    .build(),
            ),
            Some(label_factory()),
        );
        popup_queue_view.set_single_click_activate(false);
        let popup_queue_scroll = scroller(&popup_queue_view);
        popup_queue_scroll.set_size_request(400, 300);
        let queue_summary = text("Playback queue", "music-title");
        let queue_empty = text("No songs queued yet", "music-detail");
        let popup_contents = gtk::Box::new(gtk::Orientation::Vertical, 8);
        popup_contents.append(&queue_summary);
        popup_contents.append(&text(
            "Drag to arrange · double-click to play",
            "music-detail",
        ));
        popup_contents.append(&queue_empty);
        popup_contents.append(&popup_queue_scroll);
        let queue_popover = gtk::Popover::builder().child(&popup_contents).build();
        queue_popover.add_css_class("music-queue");
        let queue_button = gtk::MenuButton::builder()
            .popover(&queue_popover)
            .tooltip_text("Show the playback queue without leaving your files")
            .build();
        queue_button.add_css_class("music-control");
        let queue_button_contents = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        queue_button_contents.append(&assets::primary_icon(icons::LIST, 18));
        queue_button_contents.append(&text("Queue", "music-title"));
        queue_button.set_child(Some(&queue_button_contents));
        bar.append(&queue_button);
        let transport_options_contents = gtk::Box::new(gtk::Orientation::Vertical, 6);
        transport_options_contents.append(&stop);
        let transport_options_popover = gtk::Popover::builder()
            .child(&transport_options_contents)
            .build();
        transport_options_popover.add_css_class("music-queue");
        transport_options_popover.add_css_class("music-actions-popover");
        let transport_options = gtk::MenuButton::builder()
            .popover(&transport_options_popover)
            .tooltip_text("More playback actions · Stop playback")
            .build();
        transport_options.add_css_class("music-control");
        transport_options.add_css_class("music-transport-more");
        transport_options.set_child(Some(&control_contents(icons::SLIDERS, "More")));
        transport_options
            .update_property(&[gtk::accessible::Property::Label("More playback actions")]);
        bar.append(&transport_options);
        let queue_scroll = scroller(&queue_view);
        queue_scroll.set_size_request(310, -1);
        let queue_panel = gtk::Box::new(gtk::Orientation::Vertical, 8);
        queue_panel.append(&text("QUEUE · drag to arrange", "music-detail"));
        let queue_actions = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let queue_resume = gtk::Button::with_label("Resume queue");
        queue_resume.add_css_class("music-control");
        queue_resume.set_tooltip_text(Some(
            "Resume the saved song and position; never starts automatically",
        ));
        let queue_clear = gtk::Button::with_label("Clear queue");
        queue_clear.add_css_class("music-control");
        queue_clear.set_tooltip_text(Some(
            "Remove all queued songs and stop playback; does not delete files",
        ));
        queue_actions.append(&queue_resume);
        queue_actions.append(&queue_clear);
        queue_panel.append(&queue_actions);
        queue_panel.append(&queue_scroll);
        let library_row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        library_row.set_vexpand(true);
        library_row.append(&album_scroll);
        let group_model = gtk::StringList::new(&[]);
        let group_selection = gtk::SingleSelection::new(Some(group_model.clone()));
        let group_view = gtk::ListView::new(Some(group_selection.clone()), Some(label_factory()));
        group_view.set_single_click_activate(false);
        let group_scroll = scroller(&group_view);
        group_scroll.set_size_request(210, -1);
        group_scroll.set_visible(false);
        library_row.append(&group_scroll);
        library_row.append(&track_scroll);
        library_row.append(&queue_panel);
        view.append(&library_row);
        let actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        actions.add_css_class("music-actions");
        let play_album = labeled_button(
            icons::PLAY,
            "Play album",
            "Play the displayed album from the beginning",
        );
        play_album.add_css_class("music-action-primary");
        let play_selected = gtk::Button::with_label("Play selected");
        let add = gtk::Button::with_label("Add selected to queue");
        let play_next = gtk::Button::with_label("Play selected next");
        let shuffle = gtk::ToggleButton::new();
        shuffle.set_child(Some(&assets::primary_icon(icons::SHUFFLE, 17)));
        shuffle.set_tooltip_text(Some("Shuffle playback"));
        shuffle.update_property(&[gtk::accessible::Property::Label("Shuffle playback")]);
        let repeat = button(icons::REPEAT, "Repeat: off; click to repeat all songs");
        repeat.update_property(&[gtk::accessible::Property::Label("Repeat: off")]);
        let options_contents = gtk::Box::new(gtk::Orientation::Vertical, 6);
        for control in [&play_selected, &add, &play_next] {
            control.add_css_class("music-control");
            options_contents.append(control);
        }
        shuffle.add_css_class("music-control");
        shuffle.add_css_class("music-mode-control");
        repeat.add_css_class("music-mode-control");
        let options_popover = gtk::Popover::builder().child(&options_contents).build();
        options_popover.add_css_class("music-queue");
        options_popover.add_css_class("music-actions-popover");
        let options = gtk::MenuButton::builder()
            .popover(&options_popover)
            .tooltip_text("Play selected, add to queue, or play next")
            .build();
        options.add_css_class("music-control");
        options.add_css_class("music-selection-actions");
        options.set_child(Some(&control_contents(icons::SLIDERS, "More actions")));
        actions.append(&play_album);
        actions.append(&options);
        let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        spacer.set_hexpand(true);
        actions.append(&spacer);
        actions.append(&shuffle);
        actions.append(&repeat);
        view.append(&actions);
        let waveform_status = text("Now playing waveform · click to seek", "music-detail");
        view.append(&waveform_status);
        let waveform = gtk::DrawingArea::new();
        waveform.set_content_height(84);
        waveform.set_hexpand(true);
        waveform.add_css_class("music-waveform");
        waveform.set_visible(false);
        waveform_status.set_visible(false);
        view.append(&waveform);

        let source = Rc::downgrade(&controller);
        let target = Rc::downgrade(&controller);
        let seek_control = super::seek::SeekControl::new(
            &seek,
            move || {
                source
                    .upgrade()
                    .and_then(|controller| controller.stream.borrow().clone())
                    .map(|stream| stream.upcast())
            },
            move |timestamp| {
                if let Some(controller) = target.upgrade() {
                    controller.seek(timestamp);
                }
            },
        );
        let collection_snapshot = collection_store.snapshot();
        let state = Rc::new(State {
            controller,
            displayed_revision: Cell::new(u64::MAX),
            displayed_track: RefCell::new(None),
            playing_icon: Cell::new(false),
            muted_icon: Cell::new(false),
            repeat_icon: Cell::new(Repeat::Off),
            speaker: speaker.clone(),
            bar,
            view,
            title,
            detail,
            clock,
            status,
            cover,
            cover_button,
            play,
            #[cfg(test)]
            seek,
            seek_control,
            volume,
            syncing: Cell::new(false),
            waveform,
            waveform_status,
            waveform_toggle,
            disposed: Cell::new(false),
            scan: RefCell::new(None),
            library: RefCell::new(Vec::new()),
            library_revision: Cell::new(0),
            albums: RefCell::new(Vec::new()),
            album_model,
            album_selection,
            rendering_library: Cell::new(false),
            selected_album: RefCell::new(None),
            tracks: RefCell::new(Vec::new()),
            track_model,
            track_selection,
            queue_model,
            queue_indices: RefCell::new(Vec::new()),
            queue_button,
            queue_panel,
            queue_toggle,
            queue_summary,
            queue_empty,
            queue_resume,
            queue_clear,
            shuffle,
            repeat,
            filter,
            play_album: play_album.clone(),
            selected_actions: vec![play_selected.clone(), add.clone(), play_next.clone()],
            selection_options: options.clone(),
            collection_store,
            collection_snapshot: RefCell::new(collection_snapshot),
            section: Cell::new(Section::Albums),
            navigation,
            album_scroll,
            group_scroll,
            group_model,
            group_labels: RefCell::new(Vec::new()),
            group_section: Cell::new(None),
            group_library_revision: Cell::new(u64::MAX),
            group_selection,
            groups: RefCell::new(Vec::new()),
            playlist_editor,
            collection_status,
            favorite_buttons: RefCell::new(Vec::new()),
            favorite_paths: RefCell::new(std::collections::BTreeSet::new()),
            catalog_paths: RefCell::new(std::collections::BTreeSet::new()),
            pending_observation: RefCell::new(None),
        });
        collections::connect(&state, &create_playlist, &edit_playlist);
        let weak = Rc::downgrade(&state);
        state.waveform_toggle.connect_toggled(move |toggle| {
            if let Some(state) = weak.upgrade() {
                toggle.set_child(Some(&assets::primary_icon(
                    if toggle.is_active() {
                        icons::EYE
                    } else {
                        icons::EYE_OFF
                    },
                    16,
                )));
                toggle.set_tooltip_text(Some(if toggle.is_active() {
                    "Hide the now-playing waveform"
                } else {
                    "Show the now-playing waveform"
                }));
                state.sync_controller();
            }
        });
        let weak = Rc::downgrade(&state);
        state.queue_toggle.connect_toggled(move |toggle| {
            if let Some(state) = weak.upgrade() {
                let visible = toggle.is_active();
                state.queue_panel.set_visible(visible);
                state.queue_toggle.set_child(Some(&control_contents(
                    icons::LIST,
                    if visible { "Hide queue" } else { "Show queue" },
                )));
                state.queue_toggle.set_tooltip_text(Some(if visible {
                    "Hide the queue panel; playback continues"
                } else {
                    "Show the queue panel to resume or arrange songs"
                }));
            }
        });
        for control in [&play_selected, &add, &play_next] {
            let options = options.downgrade();
            control.connect_clicked(move |_| {
                if let Some(options) = options.upgrade() {
                    options.popdown();
                }
            });
        }
        let weak = Rc::downgrade(&state);
        state.track_selection.connect_selected_notify(move |_| {
            if let Some(state) = weak.upgrade() {
                state.sync_actions();
            }
        });
        for queue_view in [&queue_view, &popup_queue_view] {
            queue_view.set_factory(Some(&queue::factory(Rc::downgrade(&state))));
            queue::install_keys(queue_view, Rc::downgrade(&state));
        }
        let weak = Rc::downgrade(&state);
        state.queue_resume.connect_clicked(move |_| {
            if let Some(state) = weak.upgrade() {
                state.toggle();
            }
        });
        let weak = Rc::downgrade(&state);
        state.queue_clear.connect_clicked(move |_| {
            if let Some(state) = weak.upgrade() {
                state.clear_queue();
            }
        });
        albums.set_factory(Some(&album_factory(Rc::downgrade(&state))));
        tracks.set_factory(Some(&track_factory(Rc::downgrade(&state))));
        context_menu::install_keys(&albums, Rc::downgrade(&state), context_menu::Kind::Album);
        context_menu::install_keys(&tracks, Rc::downgrade(&state), context_menu::Kind::Track);
        let weak = Rc::downgrade(&state);
        state.play.connect_clicked(move |_| {
            if let Some(state) = weak.upgrade() {
                state.toggle();
            }
        });
        let weak = Rc::downgrade(&state);
        next.connect_clicked(move |_| {
            if let Some(state) = weak.upgrade() {
                state.next();
            }
        });
        let weak = Rc::downgrade(&state);
        previous.connect_clicked(move |_| {
            if let Some(state) = weak.upgrade() {
                state.previous();
            }
        });
        let weak = Rc::downgrade(&state);
        stop.connect_clicked(move |_| {
            if let Some(state) = weak.upgrade() {
                state.stop();
            }
        });
        let transport_options = transport_options.downgrade();
        stop.connect_clicked(move |_| {
            if let Some(options) = transport_options.upgrade() {
                options.popdown();
            }
        });
        let weak = Rc::downgrade(&state);
        speaker.connect_clicked(move |_| {
            if let Some(state) = weak.upgrade() {
                let stream = state.stream.borrow().clone();
                if let Some(stream) = stream {
                    stream.set_muted(!stream.is_muted());
                    state.notify();
                }
            }
        });
        let weak = Rc::downgrade(&state);
        state.volume.connect_value_changed(move |volume| {
            if let Some(state) = weak.upgrade()
                && !state.syncing.get()
            {
                state.set_volume(volume.value());
            }
        });
        let weak = Rc::downgrade(&state);
        state
            .album_selection
            .connect_selected_notify(move |selection| {
                if let Some(state) = weak.upgrade()
                    && !state.rendering_library.get()
                {
                    state.browse_album(selection.selected() as usize);
                }
            });
        let weak = Rc::downgrade(&state);
        albums.connect_activate(move |_, position| {
            if let Some(state) = weak.upgrade() {
                state.play_album(position as usize);
            }
        });
        let weak = Rc::downgrade(&state);
        tracks.connect_activate(move |_, position| {
            if let Some(state) = weak.upgrade() {
                state.play_results(position as usize);
            }
        });
        for queue_view in [&queue_view, &popup_queue_view] {
            let weak = Rc::downgrade(&state);
            queue_view.connect_activate(move |_, position| {
                if let Some(state) = weak.upgrade() {
                    state.play_queue(position as usize);
                    state.queue_button.popdown();
                }
            });
        }
        let weak = Rc::downgrade(&state);
        play_album.connect_clicked(move |_| {
            if let Some(state) = weak.upgrade() {
                if state.filter.text().is_empty() && state.section.get() == Section::Albums {
                    state.play_album(state.album_selection.selected() as usize);
                } else {
                    let tracks = state.available_tracks();
                    state.replace(tracks, 0);
                }
            }
        });
        let weak = Rc::downgrade(&state);
        play_selected.connect_clicked(move |_| {
            if let Some(state) = weak.upgrade() {
                state.play_results(state.track_selection.selected() as usize);
            }
        });
        let weak = Rc::downgrade(&state);
        add.connect_clicked(move |_| {
            if let Some(state) = weak.upgrade() {
                let tracks = state.selected_tracks();
                state.queue.borrow_mut().append(tracks);
                state.queue_changed();
            }
        });
        let weak = Rc::downgrade(&state);
        play_next.connect_clicked(move |_| {
            if let Some(state) = weak.upgrade() {
                let tracks = state.selected_tracks();
                state.queue.borrow_mut().play_next(tracks);
                state.queue_changed();
            }
        });
        let weak = Rc::downgrade(&state);
        state.shuffle.connect_toggled(move |button| {
            if let Some(state) = weak.upgrade()
                && !state.syncing.get()
            {
                state.set_shuffle(button.is_active());
            }
        });
        let weak = Rc::downgrade(&state);
        state.repeat.connect_clicked(move |_| {
            if let Some(state) = weak.upgrade() {
                let repeat = match state.controller.repeat() {
                    Repeat::Off => Repeat::All,
                    Repeat::All => Repeat::One,
                    Repeat::One => Repeat::Off,
                };
                state.set_repeat(repeat);
            }
        });
        let weak = Rc::downgrade(&state);
        state.filter.connect_changed(move |_| {
            if let Some(state) = weak.upgrade() {
                state.render_tracks();
            }
        });
        let weak = Rc::downgrade(&state);
        refresh.connect_clicked(move |_| {
            if let Some(state) = weak.upgrade() {
                state.scan();
            }
        });
        let weak = Rc::downgrade(&state);
        state.view.connect_map(move |_| {
            if let Some(state) = weak.upgrade()
                && state.scan.borrow().is_none()
            {
                state.scan();
            }
        });
        let weak = Rc::downgrade(&state);
        state
            .waveform
            .set_draw_func(move |_, context, width, height| {
                let Some(state) = weak.upgrade() else {
                    return;
                };
                let progress = state.stream.borrow().as_ref().map_or(0.0, |stream| {
                    stream.timestamp() as f64 / stream.duration().max(1) as f64
                });
                let peaks = state.peaks.borrow();
                let Some((accent, muted)) = super::theme::ThemeManager::shared().waveform_colors()
                else {
                    return;
                };
                if peaks.is_empty() {
                    return;
                }
                let step = f64::from(width) / peaks.len() as f64;
                for (index, peak) in peaks.iter().enumerate() {
                    let color = if (index as f64 / peaks.len() as f64) <= progress {
                        &accent
                    } else {
                        &muted
                    };
                    context.set_source_rgba(
                        color.red().into(),
                        color.green().into(),
                        color.blue().into(),
                        0.85,
                    );
                    let amplitude = f64::from(*peak).sqrt() * f64::from(height) * 0.9;
                    context.rectangle(
                        index as f64 * step,
                        (f64::from(height) - amplitude) / 2.0,
                        step.max(1.0),
                        amplitude.max(1.0),
                    );
                    let _drawn = context.fill();
                }
            });
        let click = gtk::GestureClick::new();
        let weak = Rc::downgrade(&state);
        click.connect_pressed(move |_, _, position, _| {
            let Some(state) = weak.upgrade() else {
                return;
            };
            let stream = state.stream.borrow().clone();
            if let Some(stream) = stream
                && stream.is_seekable()
                && !stream.is_seeking()
            {
                let fraction =
                    (position / f64::from(state.waveform.width().max(1))).clamp(0.0, 1.0);
                state
                    .controller
                    .seek((stream.duration() as f64 * fraction) as i64);
            }
        });
        state.waveform.add_controller(click);
        let weak = Rc::downgrade(&state);
        glib::timeout_add_local(Duration::from_millis(100), move || {
            let Some(state) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            if state.disposed.get() {
                return glib::ControlFlow::Break;
            }
            let snapshot = state
                .scan
                .borrow_mut()
                .as_mut()
                .and_then(|scan| scan.poll());
            if let Some(snapshot) = snapshot {
                let library_notice = format!(
                    "{} tracks · {}{}",
                    snapshot.tracks.len(),
                    if snapshot.indexing {
                        "Reading library tags…"
                    } else {
                        "Library ready"
                    },
                    snapshot
                        .notice
                        .map_or(String::new(), |notice| format!(" · {notice}"))
                );
                let show_library_notice = state.notice.borrow().is_empty();
                if show_library_notice {
                    state.status.set_text(&library_notice);
                }
                if *state.library.borrow() != snapshot.tracks {
                    state.observe_library_paths(&snapshot.tracks);
                    state.library.replace(snapshot.tracks);
                    state.render_library();
                }
            }
            state.retry_observation();
            state.poll_collections();
            glib::ControlFlow::Continue
        });
        state.controller.attach(&state);
        state.sync_actions();
        Self { state }
    }

    pub fn bar(&self) -> gtk::Widget {
        self.state.bar.clone().upcast()
    }
    pub fn connect_open_player(&self, open: Rc<dyn Fn()>) {
        let weak = Rc::downgrade(&self.state);
        self.state.cover_button.connect_clicked(move |_| {
            if let Some(state) = weak.upgrade() {
                let album = state.queue.borrow().playing().map(Track::album_key);
                if let Some(album) = album {
                    state.navigation[0].1.set_active(true);
                    state.selected_album.replace(Some(album));
                    state.filter.set_text("");
                    let has_library = !state.library.borrow().is_empty();
                    if has_library {
                        state.render_library();
                    }
                }
            }
            open();
        });
    }
    pub fn view(&self) -> gtk::Widget {
        self.state.view.clone().upcast()
    }
    #[cfg(test)]
    pub fn is_playing(&self) -> bool {
        self.state
            .stream
            .borrow()
            .as_ref()
            .is_some_and(|stream| stream.is_playing())
    }
    pub(crate) fn dispose_view(&self) {
        self.state.disposed.set(true);
        self.state.scan.borrow_mut().take();
        self.state.queue_button.popdown();
    }

    #[cfg(test)]
    pub fn shutdown(&self) {
        self.controller().shutdown();
        self.dispose_view();
    }

    pub fn play_files(&self, entries: Vec<FileEntry>) {
        let tracks: Vec<_> = entries
            .into_iter()
            .filter(crate::app::music_order::is_audio)
            .filter_map(|entry| {
                Some(Track {
                    path: entry.location.native_path()?.to_owned(),
                    tags: entry.music_tags.clone().unwrap_or_default(),
                })
            })
            .collect();
        if tracks.len() == 1 {
            self.play_folder(tracks[0].path.clone());
            return;
        }
        self.state.replace(tracks, 0);
    }

    pub fn play_folder(&self, selected: PathBuf) {
        self.controller().play_folder(selected);
    }

    pub fn queue_files(&self, entries: Vec<FileEntry>) {
        let tracks = entries
            .into_iter()
            .filter(crate::app::music_order::is_audio)
            .filter_map(|entry| {
                Some(Track {
                    path: entry.location.native_path()?.to_owned(),
                    tags: entry.music_tags.clone().unwrap_or_default(),
                })
            })
            .collect();
        self.state.queue.borrow_mut().append(tracks);
        self.state.queue_changed();
    }
}

impl State {
    fn scan(self: &Rc<Self>) {
        let mut scan = self.scan.borrow_mut();
        if let Some(scan) = scan.as_ref() {
            scan.refresh();
        } else {
            scan.replace(crate::music::library::Scan::start(music_root()));
        }
    }

    fn render_library(&self) {
        self.library_revision
            .set(self.library_revision.get().wrapping_add(1));
        self.rendering_library.set(true);
        let mut albums = std::collections::BTreeMap::<PathBuf, Album>::new();
        for track in self.library.borrow().iter() {
            let album = albums.entry(track.album_key()).or_insert_with(|| Album {
                path: track.album_key(),
                title: track.album(),
                artist: track.artist().to_owned(),
                count: 0,
            });
            album.count += 1;
            if track.tags.album.is_some() {
                album.title = track.album();
            }
            if track.artist() != "Unknown artist" {
                album.artist = track.artist().to_owned();
            }
        }
        let albums: Vec<_> = albums.into_values().collect();
        let selected = self.selected_album.borrow().clone();
        let next = selected
            .as_ref()
            .and_then(|path| albums.iter().position(|album| &album.path == path))
            .or_else(|| (!albums.is_empty()).then_some(0));
        self.selected_album
            .replace(next.map(|position| albums[position].path.clone()));
        if *self.albums.borrow() != albums {
            let labels: Vec<_> = albums
                .iter()
                .map(|album| format!("{}\n{} · {} tracks", album.title, album.artist, album.count))
                .collect();
            self.albums.replace(albums);
            replace_model(&self.album_model, &labels);
        }
        self.album_selection
            .set_selected(next.map_or(gtk::INVALID_LIST_POSITION, |position| position as u32));
        self.rendering_library.set(false);
        self.render_groups();
        self.render_tracks();
    }

    fn browse_album(&self, position: usize) {
        let path = self
            .albums
            .borrow()
            .get(position)
            .map(|album| album.path.clone());
        let Some(path) = path else {
            return;
        };
        self.selected_album.replace(Some(path));
        self.filter.set_text("");
        self.render_tracks();
    }

    fn play_album(&self, position: usize) {
        let path = self
            .albums
            .borrow()
            .get(position)
            .map(|album| album.path.clone());
        let Some(path) = path else {
            return;
        };
        let tracks = album_tracks(&self.library.borrow(), &path);
        if tracks.is_empty() {
            return;
        }
        self.browse_album(position);
        self.album_selection.set_selected(position as u32);
        self.set_shuffle(false);
        self.replace(tracks, 0);
    }

    fn sync_actions(&self) {
        let filtered = !self.filter.text().is_empty() || self.section.get() != Section::Albums;
        self.play_album.set_child(Some(&control_contents(
            icons::PLAY,
            if filtered {
                "Play results"
            } else {
                "Play album"
            },
        )));
        self.play_album.set_tooltip_text(Some(if filtered {
            "Play all displayed search results"
        } else {
            "Play the full album in disc and track order; turns shuffle off"
        }));
        self.play_album
            .set_sensitive(!self.available_tracks().is_empty());
        let selected = !self.selected_tracks().is_empty();
        for control in &self.selected_actions {
            control.set_sensitive(selected);
        }
        self.selection_options.set_sensitive(selected);
    }

    fn render_tracks(&self) {
        let query = self.filter.text().to_lowercase();
        let tracks = collections::visible_tracks(
            &self.library.borrow(),
            self.section.get(),
            self.selected_album.borrow().as_deref(),
            self.current_group().as_ref(),
            &self.collection_snapshot.borrow().data,
            &query,
        );
        self.collection_hint(tracks.len());
        if *self.tracks.borrow() == tracks {
            self.sync_actions();
            return;
        }
        let selected = self
            .tracks
            .borrow()
            .get(self.track_selection.selected() as usize)
            .map(|track| track.path.clone());
        let labels: Vec<_> = tracks
            .iter()
            .map(|track| {
                format!(
                    "{}{}\n{} · {}",
                    track
                        .tags
                        .track
                        .map_or(String::new(), |number| format!("{number:02}  ")),
                    track.title(),
                    track.artist(),
                    track.album()
                )
            })
            .collect();
        let next_selection =
            selected.and_then(|path| tracks.iter().position(|track| track.path == path));
        self.tracks.replace(tracks);
        replace_model(&self.track_model, &labels);
        if let Some(position) = next_selection {
            self.track_selection.set_selected(position as u32);
        }
        self.sync_actions();
    }

    fn selected_tracks(&self) -> Vec<Track> {
        self.tracks
            .borrow()
            .get(self.track_selection.selected() as usize)
            .filter(|track| {
                self.library
                    .borrow()
                    .iter()
                    .any(|entry| entry.path == track.path)
            })
            .cloned()
            .into_iter()
            .collect()
    }

    fn play_results(self: &Rc<Self>, position: usize) {
        let selected = self
            .tracks
            .borrow()
            .get(position)
            .map(|track| track.path.clone());
        let Some(selected) = selected else {
            return;
        };
        let tracks = self.track_playback(&selected);
        if tracks.is_empty() {
            return;
        }
        self.replace(tracks, 0);
    }

    fn play_queue(self: &Rc<Self>, position: usize) {
        self.resume_position.set(0);
        let index = self.queue_indices.borrow().get(position).copied();
        let track = index.and_then(|index| self.queue.borrow_mut().select(index));
        if let Some(track) = track {
            self.start(track);
        }
    }

    fn sync_controller(&self) {
        let track = self.current_track();
        if *self.displayed_track.borrow() != track {
            self.displayed_track.replace(track.clone());
            if let Some(track) = track.as_ref() {
                self.title.set_text(&track.title());
                super::thumbnail::set_thumbnail_or_icon(
                    &self.cover,
                    &file_entry(track),
                    icons::MUSIC,
                    42,
                    48,
                );
            } else {
                self.title.set_text("Nothing playing");
                assets::set_primary_icon(&self.cover, icons::MUSIC);
            }
        }
        let stream = self.stream.borrow().clone();
        let error = stream.as_ref().and_then(|stream| stream.error());
        if let Some(error) = error {
            self.detail
                .set_text(&format!("Playback unavailable: {error}"));
            self.detail.set_tooltip_text(Some(&error.to_string()));
        } else {
            self.detail.set_tooltip_text(None);
            self.detail.set_text(&track.map_or_else(
                || "Choose a track in Music".into(),
                |track| format!("{} · {}", track.artist(), track.album()),
            ));
        }
        let notice = self.notice.borrow().clone();
        if !notice.is_empty() {
            self.status.set_text(&notice);
        }
        let waveform_notice = self.waveform_notice.borrow().clone();
        self.waveform_status.set_text(&waveform_notice);
        let has_waveform = self.waveform_toggle.is_active()
            && stream
                .as_ref()
                .is_some_and(|stream| stream.is_prepared() && stream.error().is_none())
            && !self.peaks.borrow().is_empty();
        self.waveform.set_visible(has_waveform);
        self.waveform_status.set_visible(has_waveform);
        self.syncing.set(true);
        self.volume.set_value(self.controller.volume());
        self.shuffle.set_active(self.controller.shuffle());
        let repeat = self.controller.repeat();
        let repeat_label = match repeat {
            Repeat::Off => "Repeat: off",
            Repeat::All => "Repeat: all",
            Repeat::One => "Repeat: one",
        };
        self.repeat
            .update_property(&[gtk::accessible::Property::Label(repeat_label)]);
        self.repeat.set_tooltip_text(Some(match repeat {
            Repeat::Off => "Repeat: off; click to repeat all songs",
            Repeat::All => "Repeat: all; click to repeat this song",
            Repeat::One => "Repeat: one; click to turn repeat off",
        }));
        if self.repeat_icon.replace(repeat) != repeat {
            self.repeat.set_child(Some(&assets::primary_icon(
                if repeat == Repeat::One {
                    icons::REPEAT_ONE
                } else {
                    icons::REPEAT
                },
                17,
            )));
        }
        if repeat == Repeat::Off {
            self.repeat.remove_css_class("repeat-enabled");
        } else {
            self.repeat.add_css_class("repeat-enabled");
        }
        self.syncing.set(false);
        let muted = stream.as_ref().is_some_and(|stream| stream.is_muted());
        if self.muted_icon.replace(muted) != muted {
            self.speaker.set_child(Some(&assets::primary_icon(
                if muted { icons::MUTED } else { icons::VOLUME },
                17,
            )));
        }
        self.bar.set_visible(self.transport_visible.get());
        if self.displayed_revision.replace(self.queue_revision.get()) != self.queue_revision.get() {
            self.render_queue();
        }
        self.sync_transport();
    }

    fn render_queue(&self) {
        let labels: Vec<_> = {
            let queue = self.queue.borrow();
            let ordered = queue.ordered();
            self.queue_indices
                .replace(ordered.iter().map(|(index, _)| *index).collect());
            ordered
                .into_iter()
                .map(|(index, track)| {
                    format!(
                        "{}{}\n{}",
                        if Some(index) == queue.current {
                            "▶  "
                        } else {
                            ""
                        },
                        track.title(),
                        track.artist()
                    )
                })
                .collect()
        };
        self.queue_summary
            .set_text(&format!("Playback queue · {} songs", labels.len()));
        self.queue_empty.set_visible(labels.is_empty());
        let can_resume = !labels.is_empty()
            && self.stream.borrow().as_ref().is_none_or(|stream| {
                stream.is_prepared() && !stream.is_playing() || stream.error().is_some()
            });
        self.queue_resume.set_sensitive(can_resume);
        self.queue_clear.set_sensitive(!labels.is_empty());
        replace_model(&self.queue_model, &labels);
    }

    fn sync_transport(&self) {
        let stream = self.stream.borrow().clone();
        let playing = stream.as_ref().is_some_and(|stream| stream.is_playing());
        let can_resume = !self.queue.borrow().tracks.is_empty()
            && stream.as_ref().is_none_or(|stream| {
                stream.is_prepared() && !stream.is_playing() || stream.error().is_some()
            });
        self.queue_resume.set_sensitive(can_resume);
        self.queue_resume.set_label(
            if stream
                .as_ref()
                .is_some_and(|stream| stream.error().is_some())
            {
                "Retry song"
            } else {
                "Resume queue"
            },
        );
        self.syncing.set(true);
        self.seek_control.sync();
        if let Some(stream) = stream {
            self.play
                .set_tooltip_text(Some(if stream.error().is_some() {
                    "Retry this song"
                } else {
                    "Play / pause music"
                }));
            self.clock.set_text(&format!(
                "{} / {}",
                time_label(stream.timestamp()),
                time_label(stream.duration())
            ));
        } else {
            self.clock.set_text("0:00 / 0:00");
        }
        if self.playing_icon.replace(playing) != playing {
            self.play.set_child(Some(&assets::primary_icon(
                if playing { icons::PAUSE } else { icons::PLAY },
                17,
            )));
        }
        self.syncing.set(false);
        self.waveform.queue_draw();
    }
}

fn music_root() -> PathBuf {
    glib::user_special_dir(glib::UserDirectory::Music)
        .unwrap_or_else(|| glib::home_dir().join("Music"))
}

fn read_folder_queue(
    selected: &std::path::Path,
    cancellation: &Cancellation,
) -> Result<Vec<Track>, String> {
    let folder = if selected.is_dir() {
        selected
    } else {
        selected.parent().ok_or("Song has no parent folder")?
    };
    let entries = std::fs::read_dir(folder).map_err(|error| error.to_string())?;
    let mut tracks = Vec::new();
    for (index, entry) in entries.enumerate() {
        if cancellation.is_cancelled() {
            return Err("Queue preparation cancelled".into());
        }
        if index >= 20_000 {
            return Err("Folder exceeds the 20,000-entry queue budget".into());
        }
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        if path.is_file() {
            if crate::services::classify_by_name(path.as_os_str())
                == crate::services::FormatFamily::Audio
            {
                let tags =
                    crate::adapters::music_metadata::read_tags(&path, cancellation).unwrap_or_default();
                tracks.push(Track { path, tags });
            }
        } else if path.is_dir() && selected.is_dir() {
            if let Ok(sub_entries) = std::fs::read_dir(&path) {
                for sub_entry in sub_entries.flatten() {
                    let sub_path = sub_entry.path();
                    if sub_path.is_file()
                        && crate::services::classify_by_name(sub_path.as_os_str())
                            == crate::services::FormatFamily::Audio
                    {
                        let tags = crate::adapters::music_metadata::read_tags(&sub_path, cancellation)
                            .unwrap_or_default();
                        tracks.push(Track { path: sub_path, tags });
                    }
                }
            }
        }
    }
    Ok(crate::music::folder_queue(tracks, selected))
}

fn file_entry(track: &Track) -> FileEntry {
    FileEntry(Rc::new(FileEntryInner {
        location: Location::local(&track.path),
        native_name: track.path.file_name().unwrap_or_default().to_owned(),
        display_name: track.title(),
        kind: EntryKind::File,
        size: MetadataValue::Unknown,
        modified_unix_seconds: MetadataValue::Unknown,
        music_tags: Some(track.tags.clone()),
    }))
}

fn text(value: &str, class: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(value));
    label.add_css_class(class);
    label.set_xalign(0.0);
    label.set_ellipsize(gtk::pango::EllipsizeMode::End);
    label
}

fn button(icon: &str, tooltip: &str) -> gtk::Button {
    let button = gtk::Button::builder().tooltip_text(tooltip).build();
    button.set_child(Some(&assets::primary_icon(icon, 17)));
    button.add_css_class("music-control");
    button
}

fn control_contents(icon: &str, label: &str) -> gtk::Box {
    let contents = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    contents.append(&assets::primary_icon(icon, 17));
    contents.append(&gtk::Label::new(Some(label)));
    contents
}

fn labeled_button(icon: &str, label: &str, tooltip: &str) -> gtk::Button {
    let button = button(icon, tooltip);
    button.set_child(Some(&control_contents(icon, label)));
    button
}

fn album_tracks(library: &[Track], album: &std::path::Path) -> Vec<Track> {
    let mut tracks: Vec<_> = library
        .iter()
        .filter(|track| track.album_key() == album)
        .cloned()
        .collect();
    tracks.sort_by(crate::music::compare_tracks);
    tracks
}

fn scroller(child: &impl IsA<gtk::Widget>) -> gtk::ScrolledWindow {
    gtk::ScrolledWindow::builder()
        .child(child)
        .vexpand(true)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build()
}

fn label_factory() -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(|_, item| {
        if let Some(item) = item.downcast_ref::<gtk::ListItem>() {
            let label = text("", "music-row");
            label.set_wrap(false);
            item.set_child(Some(&label));
        }
    });
    factory.connect_bind(|_, item| {
        if let Some(item) = item.downcast_ref::<gtk::ListItem>()
            && let Some(value) = item.item().and_downcast::<gtk::StringObject>()
            && let Some(label) = item.child().and_downcast::<gtk::Label>()
        {
            label.set_text(&value.string());
        }
    });
    factory
}

fn album_factory(state: std::rc::Weak<State>) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    let setup_state = state.clone();
    factory.connect_setup(move |_, item| {
        if let Some(item) = item.downcast_ref::<gtk::ListItem>() {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let cover = assets::primary_icon(icons::MUSIC, 42);
            cover.set_size_request(48, 48);
            let label = text("", "music-row");
            label.set_hexpand(true);
            row.append(&cover);
            row.append(&label);
            context_menu::install_row(&row, item, setup_state.clone(), context_menu::Kind::Album);
            item.set_child(Some(&row));
        }
    });
    factory.connect_bind(move |_, item| {
        let Some(state) = state.upgrade() else {
            return;
        };
        let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let album = state.albums.borrow().get(item.position() as usize).cloned();
        let Some(album) = album else {
            return;
        };
        let Some(row) = item.child().and_downcast::<gtk::Box>() else {
            return;
        };
        let Some(cover) = row.first_child().and_downcast::<gtk::Image>() else {
            return;
        };
        if let Some(label) = cover.next_sibling().and_downcast::<gtk::Label>() {
            label.set_text(&format!(
                "{}\n{} · {} tracks",
                album.title, album.artist, album.count
            ));
        }
        let entry = FileEntry(Rc::new(FileEntryInner {
            location: Location::local(&album.path),
            native_name: album.path.file_name().unwrap_or_default().to_owned(),
            display_name: album.title,
            kind: EntryKind::Directory,
            size: MetadataValue::Unknown,
            modified_unix_seconds: MetadataValue::Unknown,
            music_tags: None,
        }));
        super::thumbnail::set_thumbnail_or_icon(&cover, &entry, icons::MUSIC, 42, 48);
    });
    factory.connect_unbind(|_, item| {
        if let Some(item) = item.downcast_ref::<gtk::ListItem>()
            && let Some(row) = item.child().and_downcast::<gtk::Box>()
            && let Some(cover) = row.first_child().and_downcast::<gtk::Image>()
        {
            context_menu::dismiss(row.upcast_ref());
            super::thumbnail::show_fallback_icon(&cover, icons::MUSIC, 42);
        }
    });
    factory
}

fn track_factory(state: std::rc::Weak<State>) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    let setup_state = state.clone();
    factory.connect_setup(move |_, item| {
        if let Some(item) = item.downcast_ref::<gtk::ListItem>() {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            let label = text("", "music-row");
            label.set_hexpand(true);
            row.append(&label);
            let favorite = gtk::ToggleButton::new();
            favorite.set_child(Some(&assets::primary_icon(icons::HEART, 16)));
            favorite.add_css_class("music-control");
            favorite.add_css_class("music-favorite");
            favorite.set_tooltip_text(Some("Toggle favorite; never modifies audio tags"));
            favorite.update_property(&[gtk::accessible::Property::Label("Favorite song")]);
            let weak = setup_state.clone();
            let weak_item = item.downgrade();
            favorite.connect_clicked(move |control| {
                if let (Some(state), Some(item)) = (weak.upgrade(), weak_item.upgrade())
                    && let Some(track) = state.tracks.borrow().get(item.position() as usize)
                {
                    state.collection_command(CollectionCommand::Favorite(
                        vec![track.path.clone()],
                        control.is_active(),
                    ));
                }
            });
            row.append(&favorite);
            context_menu::install_row(&row, item, setup_state.clone(), context_menu::Kind::Track);
            item.set_child(Some(&row));
        }
    });
    let bind_state = state.clone();
    factory.connect_bind(move |_, item| {
        let (Some(state), Some(item)) =
            (bind_state.upgrade(), item.downcast_ref::<gtk::ListItem>())
        else {
            return;
        };
        let Some(row) = item.child().and_downcast::<gtk::Box>() else {
            return;
        };
        let Some(track) = state.tracks.borrow().get(item.position() as usize).cloned() else {
            return;
        };
        if let Some(value) = item.item().and_downcast::<gtk::StringObject>()
            && let Some(label) = row.first_child().and_downcast::<gtk::Label>()
        {
            label.set_text(&value.string());
            label.set_tooltip_text(Some(&track.path.to_string_lossy()));
        }
        if let Some(favorite) = row.last_child().and_downcast::<gtk::ToggleButton>() {
            let active = state.favorite_paths.borrow().contains(&track.path);
            favorite.set_active(active);
            if let Some(image) = favorite.child().and_downcast::<gtk::Image>() {
                assets::set_primary_icon(
                    &image,
                    if active {
                        icons::HEART_SELECTED
                    } else {
                        icons::HEART
                    },
                );
            }
            favorite.update_property(&[gtk::accessible::Property::Label(if active {
                "Remove song from favorites"
            } else {
                "Add song to favorites"
            })]);
            let mut buttons = state.favorite_buttons.borrow_mut();
            buttons.retain(|(button, _)| button.upgrade().is_some_and(|button| button != favorite));
            buttons.push((favorite.downgrade(), track.path));
        }
    });
    factory.connect_unbind(|_, item| {
        if let Some(item) = item.downcast_ref::<gtk::ListItem>()
            && let Some(row) = item.child()
        {
            context_menu::dismiss(&row);
        }
    });
    factory
}

fn replace_model(model: &gtk::StringList, values: &[String]) {
    let old_len = model.n_items();
    let new_len = values.len() as u32;
    let min_len = old_len.min(new_len);
    for i in 0..min_len {
        let new_str = &values[i as usize];
        if model.string(i).as_deref() != Some(new_str.as_str()) {
            model.splice(i, 1, &[new_str.as_str()]);
        }
    }
    if old_len > new_len {
        model.splice(new_len, old_len - new_len, &[]);
    } else if new_len > old_len {
        let added: Vec<&str> = values[(old_len as usize)..].iter().map(String::as_str).collect();
        model.splice(old_len, 0, &added);
    }
}

fn time_label(microseconds: i64) -> String {
    let seconds = microseconds.max(0) / 1_000_000;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

#[cfg(test)]
mod tests;
