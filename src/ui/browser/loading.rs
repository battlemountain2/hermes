use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    time::Duration,
};

use gtk::{glib, prelude::*};

const LOADING_DELAY: Duration = Duration::from_millis(150);

#[derive(Clone)]
pub(in crate::ui) struct LoadingTransition(Rc<State>);

struct State {
    stack: glib::WeakRef<gtk::Stack>,
    spinner: glib::WeakRef<gtk::Spinner>,
    pending: RefCell<Option<glib::SourceId>>,
    generation: Cell<u64>,
}

impl LoadingTransition {
    pub(in crate::ui) fn new(stack: &gtk::Stack, spinner: &gtk::Spinner) -> Self {
        stack.set_transition_type(gtk::StackTransitionType::None);
        stack.add_named(
            &gtk::Box::new(gtk::Orientation::Vertical, 0),
            Some("pending"),
        );
        let transition = Self(Rc::new(State {
            stack: stack.downgrade(),
            spinner: spinner.downgrade(),
            pending: RefCell::new(None),
            generation: Cell::new(0),
        }));
        transition.begin();
        transition
    }

    pub(in crate::ui) fn begin(&self) {
        self.show("pending");
        let generation = self.0.generation.get();
        let weak = Rc::downgrade(&self.0);
        let source = glib::timeout_add_local_once(LOADING_DELAY, move || {
            let Some(state) = weak.upgrade() else {
                return;
            };
            if state.generation.get() != generation {
                return;
            }
            state.pending.take();
            let Some(stack) = state.stack.upgrade() else {
                return;
            };
            if stack.visible_child_name().as_deref() != Some("pending") {
                return;
            }
            stack.set_visible_child_name("loading");
            if let Some(spinner) = state.spinner.upgrade() {
                spinner.set_visible(true);
                if super::super::motion::animations_enabled() {
                    spinner.start();
                }
            }
        });
        self.0.pending.replace(Some(source));
    }

    pub(in crate::ui) fn show(&self, name: &str) {
        self.0
            .generation
            .set(self.0.generation.get().wrapping_add(1));
        if let Some(source) = self.0.pending.take() {
            source.remove();
        }
        if let Some(spinner) = self.0.spinner.upgrade() {
            spinner.stop();
            spinner.set_visible(false);
        }
        if let Some(stack) = self.0.stack.upgrade() {
            stack.set_visible_child_name(name);
        }
    }
}

impl Drop for State {
    fn drop(&mut self) {
        if let Some(source) = self.pending.get_mut().take() {
            source.remove();
        }
    }
}

#[cfg(test)]
mod tests;
