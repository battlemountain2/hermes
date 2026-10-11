use super::*;

impl State {
    fn queue_index(&self, item: &gtk::ListItem) -> Option<usize> {
        self.queue_indices
            .borrow()
            .get(item.position() as usize)
            .copied()
    }

    fn drag_token(&self, index: usize) -> String {
        format!(
            "hermes-queue:{}:{}:{index}",
            self.drag_identity,
            self.queue_revision.get()
        )
    }

    pub(super) fn drag_index(&self, value: &str) -> Option<usize> {
        let mut parts = value.strip_prefix("hermes-queue:")?.split(':');
        let player: u64 = parts.next()?.parse().ok()?;
        let revision: u64 = parts.next()?.parse().ok()?;
        let index: usize = parts.next()?.parse().ok()?;
        (parts.next().is_none()
            && player == self.drag_identity
            && revision == self.queue_revision.get()
            && index < self.queue.borrow().tracks.len())
        .then_some(index)
    }
}

fn clear_drop(row: &gtk::Widget) {
    row.remove_css_class("queue-drop-before");
    row.remove_css_class("queue-drop-after");
}

pub(super) fn factory(state: std::rc::Weak<State>) -> gtk::SignalListItemFactory {
    let factory = gtk::SignalListItemFactory::new();
    let weak = state.clone();
    factory.connect_setup(move |_, object| {
        let Some(item) = object.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        row.add_css_class("music-queue-row");
        let handle = assets::primary_icon(icons::LIST, 14);
        handle.set_tooltip_text(Some("Drag to reorder this song"));
        handle.set_cursor_from_name(Some("grab"));
        let label = text("", "music-row");
        label.set_hexpand(true);
        label.set_max_width_chars(24);
        row.append(&handle);
        row.append(&label);
        for (icon, tooltip, direction) in [
            (icons::ARROW_UP, "Move up (Alt+Up)", -1),
            (icons::ARROW_DOWN, "Move down (Alt+Down)", 1),
            (
                icons::X,
                "Remove from queue (Delete); does not delete the file",
                0,
            ),
        ] {
            let control = button(icon, tooltip);
            control.add_css_class("music-queue-action");
            let weak = weak.clone();
            let weak_item = item.downgrade();
            control.connect_clicked(move |_| {
                let (Some(state), Some(item)) = (weak.upgrade(), weak_item.upgrade()) else {
                    return;
                };
                let Some(index) = state.queue_index(&item) else {
                    return;
                };
                let position = item.position() as usize;
                match direction {
                    -1 if position > 0 => {
                        state.move_queue(index, position - 1);
                    }
                    1 => {
                        state.move_queue(index, position + 2);
                    }
                    0 => state.remove_queue(index),
                    _ => {}
                }
            });
            row.append(&control);
        }
        let source = gtk::DragSource::builder()
            .actions(gtk::gdk::DragAction::MOVE)
            .build();
        let weak_prepare = weak.clone();
        let weak_item = item.downgrade();
        let weak_row = row.downgrade();
        source.connect_prepare(move |source, _, _| {
            let (Some(state), Some(item)) = (weak_prepare.upgrade(), weak_item.upgrade()) else {
                return None;
            };
            let index = state.queue_index(&item)?;
            if let Some(row) = weak_row.upgrade() {
                source.set_icon(
                    Some(&gtk::WidgetPaintable::new(Some(&row))),
                    12,
                    row.height() / 2,
                );
            }
            Some(gtk::gdk::ContentProvider::for_value(
                &state.drag_token(index).to_value(),
            ))
        });
        handle.add_controller(source);
        let target = gtk::DropTarget::new(String::static_type(), gtk::gdk::DragAction::MOVE);
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
        let weak_drop = weak.clone();
        let weak_item = item.downgrade();
        let weak_row = row.downgrade();
        target.connect_drop(move |_, value, _, vertical| {
            let (Some(state), Some(item), Some(row)) =
                (weak_drop.upgrade(), weak_item.upgrade(), weak_row.upgrade())
            else {
                return false;
            };
            clear_drop(row.upcast_ref());
            let Some(index) = value
                .get::<String>()
                .ok()
                .and_then(|value| state.drag_index(&value))
            else {
                return false;
            };
            if state.queue_index(&item).is_none() {
                return false;
            }
            let destination =
                item.position() as usize + usize::from(vertical >= f64::from(row.height()) / 2.0);
            state.move_queue(index, destination)
        });
        row.add_controller(target);
        item.set_child(Some(&row));
    });
    factory.connect_bind(move |_, object| {
        let Some(item) = object.downcast_ref::<gtk::ListItem>() else {
            return;
        };
        let Some(row) = item.child().and_downcast::<gtk::Box>() else {
            return;
        };
        clear_drop(row.upcast_ref());
        if let Some(value) = item.item().and_downcast::<gtk::StringObject>()
            && let Some(label) = row
                .first_child()
                .and_then(|handle| handle.next_sibling())
                .and_downcast::<gtk::Label>()
        {
            label.set_text(&value.string());
            label.set_tooltip_text(Some(&value.string()));
        }
        if let Some(state) = state.upgrade() {
            let current = state
                .queue_index(item)
                .is_some_and(|index| state.queue.borrow().current == Some(index));
            if current {
                row.add_css_class("queue-current");
            } else {
                row.remove_css_class("queue-current");
            }
            let up = row
                .first_child()
                .and_then(|child| child.next_sibling())
                .and_then(|child| child.next_sibling());
            if let Some(up) = up {
                up.set_sensitive(item.position() > 0);
                if let Some(down) = up.next_sibling() {
                    let can_move_down =
                        (item.position() as usize + 1) < state.queue_indices.borrow().len();
                    down.set_sensitive(can_move_down);
                }
            }
        }
    });
    factory.connect_unbind(|_, object| {
        if let Some(item) = object.downcast_ref::<gtk::ListItem>()
            && let Some(row) = item.child()
        {
            clear_drop(&row);
        }
    });
    factory
}

pub(super) fn install_keys(view: &gtk::ListView, state: std::rc::Weak<State>) {
    let keys = gtk::EventControllerKey::new();
    keys.set_name(Some("hermes-queue-keys"));
    let weak_view = view.downgrade();
    keys.connect_key_pressed(move |_, key, _, modifiers| {
        let (Some(state), Some(view)) = (state.upgrade(), weak_view.upgrade()) else {
            return glib::Propagation::Proceed;
        };
        let Some(selection) = view.model().and_downcast::<gtk::SingleSelection>() else {
            return glib::Propagation::Proceed;
        };
        let position = selection.selected() as usize;
        let Some(index) = state.queue_indices.borrow().get(position).copied() else {
            return glib::Propagation::Proceed;
        };
        if key == gtk::gdk::Key::Delete && modifiers.is_empty() {
            state.remove_queue(index);
            let remaining = state.queue_indices.borrow().len();
            selection.set_selected(position.min(remaining.saturating_sub(1)) as u32);
            return glib::Propagation::Stop;
        }
        if modifiers == gtk::gdk::ModifierType::ALT_MASK {
            let destination = match key {
                gtk::gdk::Key::Up if position > 0 => Some((position - 1, position - 1)),
                gtk::gdk::Key::Down => Some((position + 2, position + 1)),
                _ => None,
            };
            if let Some((destination, selected)) = destination {
                if state.move_queue(index, destination) {
                    selection.set_selected(selected as u32);
                }
                return glib::Propagation::Stop;
            }
        }
        glib::Propagation::Proceed
    });
    view.add_controller(keys);
}
