use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Duration;

use gtk::{glib, prelude::*};

use super::{DateFormat, Value, date_format, format_absolute, format_value};

thread_local! {
    static REGISTRY: RefCell<Registry> = RefCell::new(Registry::default());
}

#[derive(Default)]
struct Registry {
    labels: HashMap<usize, Binding>,
    mapped_dates: HashSet<usize>,
    epoch: Rc<()>,
    timer: Option<Timer>,
}

struct Binding {
    label: glib::WeakRef<gtk::Label>,
    identity: Rc<()>,
    revision: Rc<()>,
    value: Option<Value>,
    date_tooltip: Option<String>,
}

struct Timer {
    source: glib::SourceId,
    identity: Rc<()>,
}

struct Update {
    key: usize,
    label: glib::WeakRef<gtk::Label>,
    identity: Rc<()>,
    revision: Rc<()>,
    epoch: Rc<()>,
    value: Option<Value>,
}

impl Registry {
    fn update(&self, key: usize) -> Option<Update> {
        self.labels.get(&key).map(|binding| Update {
            key,
            label: binding.label.clone(),
            identity: binding.identity.clone(),
            revision: binding.revision.clone(),
            epoch: self.epoch.clone(),
            value: binding.value,
        })
    }

    fn is_current(&self, update: &Update) -> bool {
        (update.value.is_none() || Rc::ptr_eq(&self.epoch, &update.epoch))
            && self.labels.get(&update.key).is_some_and(|binding| {
                Rc::ptr_eq(&binding.identity, &update.identity)
                    && Rc::ptr_eq(&binding.revision, &update.revision)
            })
    }

    fn has_identity(&self, key: usize, identity: &Rc<()>) -> bool {
        self.labels
            .get(&key)
            .is_some_and(|binding| Rc::ptr_eq(&binding.identity, identity))
    }

    fn has_timer(&self, identity: &Rc<()>) -> bool {
        self.timer
            .as_ref()
            .is_some_and(|timer| Rc::ptr_eq(&timer.identity, identity))
    }
}

pub(crate) fn bind(label: &gtk::Label, value: Value) {
    let update = retarget(label, Some(value));
    sync_timer();
    render(update, "");
}

pub(crate) fn clear(label: &gtk::Label, fallback: &str) {
    let update = retarget(label, None);
    sync_timer();
    render(update, fallback);
}

fn retarget(label: &gtk::Label, value: Option<Value>) -> Update {
    let key = label.as_ptr() as usize;
    let is_mapped = label.is_mapped();
    let (update, new_identity) = REGISTRY.with_borrow_mut(|registry| {
        let mut new_identity = None;
        let binding = registry.labels.entry(key).or_insert_with(|| {
            let identity = Rc::new(());
            new_identity = Some(identity.clone());
            Binding {
                label: label.downgrade(),
                identity,
                revision: Rc::new(()),
                value: None,
                date_tooltip: None,
            }
        });
        binding.revision = Rc::new(());
        binding.value = value;
        if is_mapped && matches!(value, Some(Value::Timestamp(_))) {
            registry.mapped_dates.insert(key);
        } else {
            registry.mapped_dates.remove(&key);
        }
        (registry.update(key).unwrap(), new_identity)
    });
    if let Some(identity) = new_identity {
        let map_identity = identity.clone();
        label.connect_map(move |label| mapping_changed(label, key, &map_identity));
        let unmap_identity = identity.clone();
        label.connect_unmap(move |label| mapping_changed(label, key, &unmap_identity));
        label.connect_destroy(move |_| destroyed(key, &identity));
    }
    update
}

fn render(update: Update, fallback: &str) {
    let Some(label) = update.label.upgrade() else {
        return;
    };
    if !REGISTRY.with_borrow(|registry| registry.is_current(&update)) {
        return;
    }
    let text = update
        .value
        .map(format_value)
        .unwrap_or_else(|| fallback.to_owned());
    let tooltip = match update.value {
        Some(Value::Timestamp(seconds)) => Some(format_absolute(seconds)),
        _ => None,
    };
    let previous_tooltip = REGISTRY.with_borrow_mut(|registry| {
        if !registry.is_current(&update) {
            return None;
        }
        let binding = registry.labels.get_mut(&update.key).unwrap();
        Some(std::mem::replace(
            &mut binding.date_tooltip,
            tooltip.clone(),
        ))
    });
    let Some(previous_tooltip) = previous_tooltip else {
        return;
    };
    let current_tooltip = label.tooltip_text();
    if let Some(tooltip) = tooltip {
        if current_tooltip.as_deref() != Some(tooltip.as_str()) {
            label.set_tooltip_text(Some(&tooltip));
        }
    } else if previous_tooltip.is_some()
        && current_tooltip.as_deref() == previous_tooltip.as_deref()
    {
        label.set_tooltip_text(None);
    }
    if REGISTRY.with_borrow(|registry| registry.is_current(&update)) && label.text() != text {
        label.set_text(&text);
    }
}

fn mapping_changed(label: &gtk::Label, key: usize, identity: &Rc<()>) {
    let update = REGISTRY.with_borrow_mut(|registry| {
        if !registry.has_identity(key, identity) {
            return None;
        }
        if label.is_mapped() && matches!(registry.labels[&key].value, Some(Value::Timestamp(_))) {
            registry.mapped_dates.insert(key);
            registry.update(key)
        } else {
            registry.mapped_dates.remove(&key);
            None
        }
    });
    sync_timer();
    if let Some(update) = update {
        render(update, "");
    }
}

fn destroyed(key: usize, identity: &Rc<()>) {
    REGISTRY.with_borrow_mut(|registry| {
        if registry.has_identity(key, identity) {
            registry.labels.remove(&key);
            registry.mapped_dates.remove(&key);
        }
    });
    sync_timer();
}

pub(super) fn refresh() {
    let updates = REGISTRY.with_borrow_mut(|registry| {
        registry.epoch = Rc::new(());
        registry
            .labels
            .iter()
            .filter(|(_, binding)| binding.value.is_some())
            .filter_map(|(&key, _)| registry.update(key))
            .collect::<Vec<_>>()
    });
    sync_timer();
    for update in updates {
        render(update, "");
    }
}

fn sync_timer() {
    let relative = date_format() == DateFormat::Relative;
    let (start, stopped) = REGISTRY.with_borrow_mut(|registry| {
        let needed = relative && !registry.mapped_dates.is_empty();
        let stopped = if needed { None } else { registry.timer.take() };
        (needed && registry.timer.is_none(), stopped)
    });
    if let Some(timer) = stopped {
        timer.source.remove();
    }
    if start {
        let identity = Rc::new(());
        let callback_identity = identity.clone();
        let source =
            glib::timeout_add_local(Duration::from_secs(60), move || tick(&callback_identity));
        REGISTRY.with_borrow_mut(|registry| {
            registry.timer = Some(Timer { source, identity });
        });
    }
}

fn tick(identity: &Rc<()>) -> glib::ControlFlow {
    let updates = REGISTRY.with_borrow(|registry| {
        if !registry.has_timer(identity) {
            return Vec::new();
        }
        registry
            .mapped_dates
            .iter()
            .filter_map(|&key| registry.update(key))
            .collect::<Vec<_>>()
    });
    for update in updates {
        if !REGISTRY.with_borrow(|registry| registry.has_timer(identity)) {
            return glib::ControlFlow::Break;
        }
        let still_mapped = REGISTRY.with_borrow(|registry| {
            registry.mapped_dates.contains(&update.key) && registry.is_current(&update)
        });
        if still_mapped {
            render(update, "");
        }
    }
    if REGISTRY.with_borrow(|registry| registry.has_timer(identity)) {
        glib::ControlFlow::Continue
    } else {
        glib::ControlFlow::Break
    }
}

#[cfg(test)]
mod tests;
