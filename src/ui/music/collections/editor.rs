use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

pub(super) struct Editor {
    state: std::rc::Weak<State>,
    window: gtk::Window,
    id: u64,
    identity: u64,
    revision: Cell<u64>,
    drag_revision: Cell<u64>,
    library_revision: Cell<u64>,
    closed: Cell<bool>,
    members: RefCell<Vec<PathBuf>>,
    member_labels: RefCell<Vec<String>>,
    model: gtk::StringList,
    selection: gtk::SingleSelection,
    catalog: RefCell<Vec<PathBuf>>,
    catalog_labels: RefCell<Vec<String>>,
    catalog_model: gtk::StringList,
    catalog_selection: gtk::SingleSelection,
    query: gtk::Entry,
    name: gtk::Entry,
    status: gtk::Label,
}

pub(super) fn open(state: &Rc<State>, id: Option<u64>) -> gtk::Window {
    let window = gtk::Window::builder()
        .title("Playlist editor")
        .default_width(750)
        .default_height(560)
        .build();
    if let Some(parent) = state.view.root().and_downcast::<gtk::Window>() {
        window.set_transient_for(Some(&parent));
        window.set_destroy_with_parent(true);
    }
    let contents = gtk::Box::new(gtk::Orientation::Vertical, 10);
    contents.add_css_class("music-playlist-editor");
    for set_margin in [
        gtk::Widget::set_margin_top,
        gtk::Widget::set_margin_bottom,
        gtk::Widget::set_margin_start,
        gtk::Widget::set_margin_end,
    ] {
        set_margin(contents.upcast_ref(), 16);
    }
    let header = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let name = gtk::Entry::builder()
        .placeholder_text("Playlist name")
        .max_length(80)
        .hexpand(true)
        .build();
    name.add_css_class("music-filter");
    name.update_property(&[gtk::accessible::Property::Label("Playlist name")]);
    header.append(&name);
    let save_name = gtk::Button::with_label(if id.is_some() {
        "Rename"
    } else {
        "Create playlist"
    });
    save_name.add_css_class("music-control");
    header.append(&save_name);
    contents.append(&header);
    let status = text(
        "Changes save locally; audio files are never edited",
        "music-detail",
    );
    status.set_wrap(true);
    contents.append(&status);
    window.set_child(Some(&contents));
    let Some(id) = id else {
        let weak = Rc::downgrade(state);
        let entry = name.clone();
        let dialog = window.downgrade();
        save_name.connect_clicked(move |_| {
            let Some(state) = weak.upgrade() else {
                return;
            };
            let name = entry.text().trim().to_owned();
            if name.is_empty() {
                entry.grab_focus();
                return;
            }
            if let Err(error) = state.collection_store.send(CollectionCommand::Create(name)) {
                status.set_text(&error);
            } else if let Some(dialog) = dialog.upgrade() {
                dialog.close();
            }
        });
        window.set_default_size(480, 140);
        window.present();
        return window;
    };
    if let Some(playlist) = state.collection_store.snapshot().data.playlist(id) {
        name.set_text(&playlist.name);
    }
    let body = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    body.set_vexpand(true);
    let members_pane = gtk::Box::new(gtk::Orientation::Vertical, 6);
    members_pane.set_hexpand(true);
    members_pane.append(&text(
        "Playlist · drag the handle to reorder",
        "music-heading",
    ));
    let model = gtk::StringList::new(&[]);
    let selection = gtk::SingleSelection::new(Some(model.clone()));
    let member_view = gtk::ListView::new(Some(selection.clone()), Some(label_factory()));
    members_pane.append(&scroller(&member_view));
    let member_actions = gtk::Box::new(gtk::Orientation::Horizontal, 4);
    let up = labeled_button(icons::ARROW_UP, "Move up", "Move selected song up · Alt+Up");
    let down = labeled_button(
        icons::ARROW_DOWN,
        "Move down",
        "Move selected song down · Alt+Down",
    );
    let remove = labeled_button(
        icons::X,
        "Remove",
        "Remove membership, never the audio file · Delete",
    );
    for control in [&up, &down, &remove] {
        member_actions.append(control);
    }
    members_pane.append(&member_actions);
    body.append(&members_pane);
    let catalog_pane = gtk::Box::new(gtk::Orientation::Vertical, 6);
    catalog_pane.set_hexpand(true);
    catalog_pane.append(&text("Add songs from the current library", "music-heading"));
    let query = gtk::Entry::builder()
        .placeholder_text("Find title, artist or album")
        .build();
    query.add_css_class("music-filter");
    catalog_pane.append(&query);
    let catalog_model = gtk::StringList::new(&[]);
    let catalog_selection = gtk::SingleSelection::new(Some(catalog_model.clone()));
    let catalog_view = gtk::ListView::new(Some(catalog_selection.clone()), Some(label_factory()));
    catalog_view.set_single_click_activate(false);
    catalog_pane.append(&scroller(&catalog_view));
    let add = labeled_button(
        icons::PLUS,
        "Add selected",
        "Add selected library song without starting playback",
    );
    catalog_pane.append(&add);
    catalog_pane.append(&text(
        "Up to 200 matches · narrow the search for more",
        "music-detail",
    ));
    body.append(&catalog_pane);
    contents.append(&body);
    let footer = gtk::Box::new(gtk::Orientation::Horizontal, 6);
    let delete = labeled_button(
        icons::TRASH,
        "Delete playlist",
        "Delete only this playlist; never delete audio files",
    );
    let close = gtk::Button::with_label("Done");
    close.add_css_class("music-control");
    footer.append(&delete);
    let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    footer.append(&spacer);
    footer.append(&close);
    contents.append(&footer);
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let editor = Rc::new(Editor {
        state: Rc::downgrade(state),
        window: window.clone(),
        id,
        identity: NEXT.fetch_add(1, Ordering::Relaxed),
        revision: Cell::new(0),
        drag_revision: Cell::new(0),
        library_revision: Cell::new(u64::MAX),
        closed: Cell::new(false),
        members: RefCell::new(Vec::new()),
        member_labels: RefCell::new(Vec::new()),
        model,
        selection,
        catalog: RefCell::new(Vec::new()),
        catalog_labels: RefCell::new(Vec::new()),
        catalog_model,
        catalog_selection,
        query,
        name,
        status,
    });
    member_view.set_factory(Some(&member_factory(Rc::downgrade(&editor))));
    for (control, offset) in [(&up, -1), (&down, 1), (&remove, 0)] {
        let weak = Rc::downgrade(&editor);
        control.connect_clicked(move |_| {
            if let Some(editor) = weak.upgrade() {
                editor.move_or_remove(offset);
            }
        });
    }
    let keys = gtk::EventControllerKey::new();
    let weak = Rc::downgrade(&editor);
    keys.connect_key_pressed(move |_, key, _, modifiers| {
        let Some(editor) = weak.upgrade() else {
            return glib::Propagation::Proceed;
        };
        let offset = match key {
            gtk::gdk::Key::Delete if modifiers.is_empty() => 0,
            gtk::gdk::Key::Up if modifiers == gtk::gdk::ModifierType::ALT_MASK => -1,
            gtk::gdk::Key::Down if modifiers == gtk::gdk::ModifierType::ALT_MASK => 1,
            _ => return glib::Propagation::Proceed,
        };
        editor.move_or_remove(offset);
        glib::Propagation::Stop
    });
    member_view.add_controller(keys);
    let weak = Rc::downgrade(&editor);
    add.connect_clicked(move |_| {
        if let Some(editor) = weak.upgrade() {
            editor.add_selected();
        }
    });
    let weak = Rc::downgrade(&editor);
    catalog_view.connect_activate(move |_, _| {
        if let Some(editor) = weak.upgrade() {
            editor.add_selected();
        }
    });
    let weak = Rc::downgrade(&editor);
    editor.query.connect_changed(move |_| {
        if let Some(editor) = weak.upgrade() {
            editor.render_catalog();
        }
    });
    let weak = Rc::downgrade(&editor);
    save_name.connect_clicked(move |_| {
        if let Some(editor) = weak.upgrade() {
            editor.send(CollectionCommand::Rename(
                editor.id,
                editor.name.text().into(),
            ));
        }
    });
    let weak = Rc::downgrade(&editor);
    delete.connect_clicked(move |_| {
        if let Some(editor) = weak.upgrade() {
            editor.send(CollectionCommand::Delete(editor.id));
        }
    });
    let dialog = window.downgrade();
    close.connect_clicked(move |_| {
        if let Some(dialog) = dialog.upgrade() {
            dialog.close();
        }
    });
    let weak = Rc::downgrade(&editor);
    window.connect_close_request(move |_| {
        if let Some(editor) = weak.upgrade() {
            editor.closed.set(true);
        }
        glib::Propagation::Proceed
    });
    editor.refresh();
    glib::timeout_add_local(Duration::from_millis(100), move || {
        if editor.closed.get() || !editor.window.is_visible() {
            return glib::ControlFlow::Break;
        }
        editor.refresh();
        glib::ControlFlow::Continue
    });
    window.present();
    window
}

impl Editor {
    fn send(&self, command: CollectionCommand) {
        if let Some(state) = self.state.upgrade() {
            if let Err(error) = state.collection_store.send(command) {
                self.status.set_text(&error);
            }
        }
    }

    fn move_or_remove(&self, offset: isize) {
        if let Some(path) = self
            .members
            .borrow()
            .get(self.selection.selected() as usize)
            .cloned()
        {
            self.send(if offset == 0 {
                CollectionCommand::Remove(self.id, vec![path])
            } else {
                CollectionCommand::Move(self.id, path, offset)
            });
        }
    }

    fn add_selected(&self) {
        if let Some(path) = self
            .catalog
            .borrow()
            .get(self.catalog_selection.selected() as usize)
            .cloned()
        {
            self.send(CollectionCommand::Add(self.id, vec![path]));
        }
    }

    fn refresh(&self) {
        let Some(state) = self.state.upgrade().filter(|state| !state.disposed.get()) else {
            self.window.close();
            return;
        };
        let snapshot = state.collection_store.snapshot();
        let library_changed = self.library_revision.replace(state.library_revision.get())
            != state.library_revision.get();
        if self.revision.replace(snapshot.revision) == snapshot.revision && !library_changed {
            return;
        }
        let Some(playlist) = snapshot.data.playlist(self.id) else {
            self.window.close();
            return;
        };
        self.status.set_text(snapshot.error.as_deref().unwrap_or(
            "Changes save locally · unavailable entries stay saved and playback skips them",
        ));
        self.window
            .set_title(Some(&format!("Edit playlist · {}", playlist.name)));
        let membership_changed = !self
            .members
            .borrow()
            .iter()
            .map(PathBuf::as_path)
            .eq(playlist.tracks.iter().filter_map(Location::native_path));
        if !membership_changed && !library_changed {
            return;
        }
        let tracks = visible_tracks(
            &state.library.borrow(),
            Section::Playlists,
            None,
            Some(&Group::Playlist(self.id)),
            &snapshot.data,
            "",
        );
        let selected = self
            .members
            .borrow()
            .get(self.selection.selected() as usize)
            .cloned();
        let paths: Vec<_> = tracks.iter().map(|track| track.path.clone()).collect();
        let labels: Vec<_> = tracks
            .iter()
            .map(|track| format!("{}\n{} · {}", track.title(), track.artist(), track.album()))
            .collect();
        let position = selected.and_then(|path| paths.iter().position(|entry| entry == &path));
        if membership_changed {
            self.drag_revision
                .set(self.drag_revision.get().wrapping_add(1));
        }
        if *self.members.borrow() != paths || *self.member_labels.borrow() != labels {
            self.members.replace(paths);
            replace_model(&self.model, &labels);
            self.member_labels.replace(labels);
            if let Some(position) = position {
                self.selection.set_selected(position as u32);
            }
        }
        if library_changed {
            self.render_catalog();
        }
    }

    fn render_catalog(&self) {
        let Some(state) = self.state.upgrade() else {
            return;
        };
        let query = self.query.text().to_lowercase();
        let library = state.library.borrow();
        let tracks: Vec<_> = library
            .iter()
            .filter(|track| {
                let haystack = format!("{} {} {}", track.title(), track.artist(), track.album())
                    .to_lowercase();
                query.split_whitespace().all(|term| haystack.contains(term))
            })
            .take(200)
            .collect();
        let labels: Vec<_> = tracks
            .iter()
            .map(|track| format!("{}\n{} · {}", track.title(), track.artist(), track.album()))
            .collect();
        let paths: Vec<_> = tracks.iter().map(|track| track.path.clone()).collect();
        if *self.catalog.borrow() == paths && *self.catalog_labels.borrow() == labels {
            return;
        }
        self.catalog.replace(paths);
        replace_model(&self.catalog_model, &labels);
        self.catalog_labels.replace(labels);
    }

    fn drag_token(&self, position: u32) -> Option<String> {
        let path = self.members.borrow().get(position as usize)?.clone();
        serde_json::to_string(&(
            self.identity,
            self.id,
            self.drag_revision.get(),
            Location::local(path),
        ))
        .ok()
    }

    fn drop_token(&self, token: &str, position: u32, after: bool) -> bool {
        if token.len() > 20_000 {
            return false;
        }
        let Ok((identity, id, revision, path)) =
            serde_json::from_str::<(u64, u64, u64, Location)>(token)
        else {
            return false;
        };
        if identity != self.identity || id != self.id || revision != self.drag_revision.get() {
            return false;
        }
        let Some(path) = path.native_path() else {
            return false;
        };
        let Some(anchor) = self.members.borrow().get(position as usize).cloned() else {
            return false;
        };
        if !self.members.borrow().iter().any(|entry| entry == path) {
            return false;
        }
        self.send(CollectionCommand::MoveBefore(
            self.id,
            path.to_owned(),
            anchor,
            after,
        ));
        true
    }
}

fn member_factory(editor: std::rc::Weak<Editor>) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    factory.connect_setup(move |_, object| {
        let Some(item) = object.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        row.add_css_class("music-playlist-row");
        let handle = assets::primary_icon(icons::LIST, 16);
        handle.set_tooltip_text(Some("Drag to reorder this playlist song"));
        row.append(&handle);
        let label = text("", "music-row");
        label.set_hexpand(true);
        row.append(&label);
        let source = gtk::DragSource::builder()
            .actions(gtk::gdk::DragAction::MOVE)
            .build();
        source.set_name(Some("hermes-playlist-drag"));
        let weak = editor.clone();
        let weak_item = item.downgrade();
        source.connect_prepare(move |_, _, _| {
            let (Some(editor), Some(item)) = (weak.upgrade(), weak_item.upgrade()) else {
                return None;
            };
            Some(gtk::gdk::ContentProvider::for_value(
                &editor.drag_token(item.position())?.to_value(),
            ))
        });
        handle.add_controller(source);
        let target = gtk::DropTarget::new(String::static_type(), gtk::gdk::DragAction::MOVE);
        target.set_name(Some("hermes-playlist-drop"));
        let weak_row = row.downgrade();
        target.connect_motion(move |_, _, vertical| {
            if let Some(row) = weak_row.upgrade() {
                clear_drop(row.upcast_ref());
                row.add_css_class(if vertical < f64::from(row.height()) / 2.0 {
                    "queue-drop-before"
                } else {
                    "queue-drop-after"
                });
            }
            gtk::gdk::DragAction::MOVE
        });
        let weak_row = row.downgrade();
        target.connect_leave(move |_| {
            if let Some(row) = weak_row.upgrade() {
                clear_drop(row.upcast_ref());
            }
        });
        let weak = editor.clone();
        let weak_item = item.downgrade();
        let weak_row = row.downgrade();
        target.connect_drop(move |_, value, _, vertical| {
            let (Some(editor), Some(item), Some(row)) =
                (weak.upgrade(), weak_item.upgrade(), weak_row.upgrade())
            else {
                return false;
            };
            clear_drop(row.upcast_ref());
            value.get::<String>().ok().is_some_and(|token| {
                editor.drop_token(
                    &token,
                    item.position(),
                    vertical >= f64::from(row.height()) / 2.0,
                )
            })
        });
        row.add_controller(target);
        item.set_child(Some(&row));
    });
    factory.connect_bind(|_, object| {
        let Some(item) = object.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let Some(row) = item.child().and_downcast::<gtk::Box>() else {
            return;
        };
        clear_drop(row.upcast_ref());
        if let Some(value) = item.item().and_downcast::<gtk::StringObject>()
            && let Some(label) = row.last_child().and_downcast::<gtk::Label>()
        {
            label.set_text(&value.string());
        }
    });
    factory
}

fn clear_drop(row: &gtk::Widget) {
    row.remove_css_class("queue-drop-before");
    row.remove_css_class("queue-drop-after");
}
