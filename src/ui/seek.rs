use std::{cell::Cell, rc::Rc};

use gtk::{gdk, glib, prelude::*};

pub(super) struct SeekControl {
    scale: glib::WeakRef<gtk::Scale>,
    stream: glib::WeakRef<gtk::MediaStream>,
    get_stream: Box<dyn Fn() -> Option<gtk::MediaStream>>,
    seek_to: Box<dyn Fn(i64)>,
    dragging: Cell<bool>,
    syncing: Cell<bool>,
    pending: Cell<Option<i64>>,
}

impl SeekControl {
    pub(super) fn new(
        scale: &gtk::Scale,
        get_stream: impl Fn() -> Option<gtk::MediaStream> + 'static,
        seek_to: impl Fn(i64) + 'static,
    ) -> Rc<Self> {
        let control = Rc::new(Self {
            scale: scale.downgrade(),
            stream: glib::WeakRef::new(),
            get_stream: Box::new(get_stream),
            seek_to: Box::new(seek_to),
            dragging: Cell::new(false),
            syncing: Cell::new(false),
            pending: Cell::new(None),
        });
        let interaction = gtk::EventControllerLegacy::new();
        interaction.set_propagation_phase(gtk::PropagationPhase::Capture);
        let state = control.clone();
        interaction.connect_event(move |_, event| {
            match event.event_type() {
                gdk::EventType::ButtonPress
                    if event
                        .downcast_ref::<gdk::ButtonEvent>()
                        .is_some_and(|event| event.button() == 1) =>
                {
                    state.begin();
                }
                gdk::EventType::TouchBegin => state.begin(),
                gdk::EventType::ButtonRelease
                    if event
                        .downcast_ref::<gdk::ButtonEvent>()
                        .is_some_and(|event| event.button() == 1) =>
                {
                    state.finish();
                }
                gdk::EventType::TouchEnd => state.finish(),
                gdk::EventType::TouchCancel | gdk::EventType::GrabBroken => state.cancel(),
                _ => {}
            }
            glib::Propagation::Proceed
        });
        scale.add_controller(interaction);
        let state = control.clone();
        scale.connect_value_changed(move |scale| {
            if state.syncing.get() {
                return;
            }
            if let Some(stream) = (state.get_stream)().filter(Self::available) {
                state
                    .pending
                    .set(Some(timestamp(scale.value(), stream.duration())));
                state.flush();
            }
        });
        let state = control.clone();
        scale.connect_unmap(move |_| state.cancel());
        control.sync();
        control
    }

    fn available(stream: &gtk::MediaStream) -> bool {
        stream.is_prepared()
            && stream.is_seekable()
            && stream.error().is_none()
            && stream.duration() > 0
    }

    fn begin(&self) {
        self.dragging.set(true);
    }

    fn finish(self: &Rc<Self>) {
        if !self.dragging.replace(false) {
            return;
        }
        let weak = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            if let Some(state) = weak.upgrade() {
                state.flush();
            }
        });
    }

    fn cancel(&self) {
        self.dragging.set(false);
        self.pending.set(None);
    }

    fn flush(&self) {
        if self.dragging.get() {
            return;
        }
        let Some(stream) = (self.get_stream)().filter(Self::available) else {
            self.pending.set(None);
            return;
        };
        if self.stream.upgrade().as_ref() != Some(&stream) {
            self.cancel();
            return;
        }
        if !stream.is_seeking()
            && let Some(target) = self.pending.take()
        {
            (self.seek_to)(target);
        }
    }

    pub(super) fn sync(&self) {
        let Some(scale) = self.scale.upgrade() else {
            self.cancel();
            return;
        };
        let stream = (self.get_stream)();
        if self.stream.upgrade().as_ref() != stream.as_ref() {
            self.cancel();
            self.stream.set(stream.as_ref());
        }
        self.syncing.set(true);
        let duration = stream.as_ref().map_or(0, |stream| stream.duration().max(0));
        scale.set_range(0.0, (duration as f64 / 1_000_000.0).max(1.0));
        if !self.dragging.get()
            && self.pending.get().is_none()
            && stream.as_ref().is_none_or(|stream| !stream.is_seeking())
        {
            scale.set_value(
                stream
                    .as_ref()
                    .map_or(0.0, |stream| stream.timestamp().max(0) as f64 / 1_000_000.0),
            );
        }
        scale.set_sensitive(stream.as_ref().is_some_and(Self::available));
        self.syncing.set(false);
        self.flush();
    }
}

fn timestamp(seconds: f64, duration: i64) -> i64 {
    if !seconds.is_finite() {
        return 0;
    }
    ((seconds.max(0.0) * 1_000_000.0) as i64).min(duration.saturating_sub(1).max(0))
}

#[cfg(test)]
mod tests;
