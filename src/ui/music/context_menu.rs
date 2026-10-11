use super::*;

#[derive(Clone, Copy)]
pub(super) enum Kind {
    Album,
    Track,
}

#[derive(Clone)]
struct Target {
    title: String,
    detail: String,
    tracks: Vec<Track>,
    playback: Vec<Track>,
    album: bool,
}

impl Target {
    fn resolve(state: &State, kind: Kind, position: u32) -> Option<Self> {
        if position == gtk::INVALID_LIST_POSITION || state.disposed.get() {
            return None;
        }
        match kind {
            Kind::Album => {
                let album = state.albums.borrow().get(position as usize)?.clone();
                let tracks = album_tracks(&state.library.borrow(), &album.path);
                if tracks.is_empty() {
                    return None;
                }
                Some(Self {
                    title: album.title,
                    detail: format!("{} · {} tracks", album.artist, tracks.len()),
                    playback: tracks.clone(),
                    tracks,
                    album: true,
                })
            }
            Kind::Track => {
                let track = state.tracks.borrow().get(position as usize)?.clone();
                Some(Self {
                    title: track.title(),
                    detail: format!("{} · {}", track.artist(), track.album()),
                    playback: crate::music::folder_queue(
                        album_tracks(&state.library.borrow(), &track.album_key()),
                        &track.path,
                    ),
                    tracks: vec![track],
                    album: false,
                })
            }
        }
    }
}

pub(super) fn install_row(
    row: &impl IsA<gtk::Widget>,
    item: &gtk::ListItem,
    state: std::rc::Weak<State>,
    kind: Kind,
) {
    let gesture = gtk::GestureClick::new();
    gesture.set_button(3);
    let item = item.downgrade();
    gesture.connect_pressed(move |gesture, _, horizontal, vertical| {
        let Some(state) = state.upgrade() else { return };
        let Some(item) = item.upgrade() else { return };
        let Some(target) = Target::resolve(&state, kind, item.position()) else {
            return;
        };
        let Some(row) = gesture.widget() else { return };
        show(&row, Rc::downgrade(&state), target, horizontal, vertical);
        gesture.set_state(gtk::EventSequenceState::Claimed);
    });
    row.add_controller(gesture);
    row.connect_unmap(|row| dismiss(row.as_ref()));
}

pub(super) fn install_keys(view: &gtk::ListView, state: std::rc::Weak<State>, kind: Kind) {
    let keys = gtk::EventControllerKey::new();
    keys.set_name(Some("hermes-music-context-keys"));
    keys.set_propagation_phase(gtk::PropagationPhase::Capture);
    keys.connect_key_pressed(move |keys, key, _, modifiers| {
        if key != gtk::gdk::Key::Menu
            && !(key == gtk::gdk::Key::F10
                && modifiers.contains(gtk::gdk::ModifierType::SHIFT_MASK))
        {
            return glib::Propagation::Proceed;
        }
        let Some(state) = state.upgrade() else {
            return glib::Propagation::Proceed;
        };
        let position = match kind {
            Kind::Album => state.album_selection.selected(),
            Kind::Track => state.track_selection.selected(),
        };
        let Some(target) = Target::resolve(&state, kind, position) else {
            return glib::Propagation::Proceed;
        };
        let Some(view) = keys.widget() else {
            return glib::Propagation::Proceed;
        };
        show(&view, Rc::downgrade(&state), target, 24.0, 24.0);
        glib::Propagation::Stop
    });
    view.add_controller(keys);
    view.connect_unmap(|view| dismiss(view.upcast_ref()));
}

pub(super) fn dismiss(widget: &gtk::Widget) {
    let mut child = widget.first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        if let Ok(popover) = current.downcast::<gtk::Popover>()
            && popover.has_css_class("music-context-menu")
        {
            popover.popdown();
            if popover.parent().is_some() {
                popover.unparent();
            }
        }
    }
}

fn show(
    anchor: &gtk::Widget,
    state: std::rc::Weak<State>,
    target: Target,
    horizontal: f64,
    vertical: f64,
) -> gtk::Popover {
    dismiss(anchor);
    let popover = gtk::Popover::new();
    popover.add_css_class("music-actions-popover");
    popover.add_css_class("music-queue");
    popover.add_css_class("music-context-menu");
    popover.set_has_arrow(false);
    popover.set_parent(anchor);
    popover.set_pointing_to(Some(&gtk::gdk::Rectangle::new(
        horizontal as i32,
        vertical as i32,
        1,
        1,
    )));
    let contents = gtk::Box::new(gtk::Orientation::Vertical, 4);
    let title = text(&target.title, "music-heading");
    title.set_max_width_chars(32);
    title.set_ellipsize(gtk::pango::EllipsizeMode::End);
    contents.append(&title);
    let detail = text(&target.detail, "music-detail");
    detail.set_max_width_chars(40);
    detail.set_ellipsize(gtk::pango::EllipsizeMode::End);
    contents.append(&detail);
    for (action, icon, label) in [
        (
            0,
            icons::PLAY,
            if target.album {
                "Play album"
            } else {
                "Play from this track"
            },
        ),
        (1, icons::ARROW_RIGHT, "Play next"),
        (
            2,
            icons::PLUS,
            if target.album {
                "Add album to queue"
            } else {
                "Add to queue"
            },
        ),
    ] {
        let control = gtk::Button::new();
        control.set_child(Some(&control_contents(icon, label)));
        control.add_css_class("music-control");
        control.update_property(&[gtk::accessible::Property::Label(label)]);
        let state = state.clone();
        let target = target.clone();
        let menu = popover.downgrade();
        control.connect_clicked(move |_| {
            if let Some(menu) = menu.upgrade() {
                menu.popdown();
            }
            let Some(state) = state.upgrade().filter(|state| !state.disposed.get()) else {
                return;
            };
            match action {
                0 => {
                    if target.album {
                        state.set_shuffle(false);
                    }
                    state.replace(target.playback.clone(), 0);
                }
                1 => {
                    state.queue.borrow_mut().play_next(target.tracks.clone());
                    state.queue_changed();
                }
                _ => {
                    state.queue.borrow_mut().append(target.tracks.clone());
                    state.queue_changed();
                }
            }
        });
        contents.append(&control);
    }
    popover.set_child(Some(&contents));
    popover.connect_closed(|menu| {
        if menu.parent().is_some() {
            menu.unparent();
        }
    });
    popover.popup();
    popover
}

#[cfg(test)]
mod tests;
