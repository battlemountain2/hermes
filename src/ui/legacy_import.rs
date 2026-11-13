// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    rc::Rc,
};

use gtk::{gio, glib, prelude::*};

use crate::paths::legacy_import::{Choice, Roots, Status};

thread_local! {
    static GATE: RefCell<Option<Rc<Gate>>> = const { RefCell::new(None) };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Idle,
    Checking,
    Waiting,
    Working,
    Ready,
}

impl Phase {
    fn busy(self) -> bool {
        matches!(self, Self::Checking | Self::Working)
    }
}

struct Gate {
    application: glib::WeakRef<gtk::Application>,
    phase: Cell<Phase>,
    pending: RefCell<VecDeque<Option<crate::file_manager::Request>>>,
    window: RefCell<Option<gtk::ApplicationWindow>>,
}

pub(super) fn present(
    application: &gtk::Application,
    request: Option<crate::file_manager::Request>,
) {
    let gate = GATE.with_borrow_mut(|slot| {
        if let Some(gate) = slot.as_ref()
            && gate.application.upgrade().as_ref() == Some(application)
        {
            return gate.clone();
        }
        let gate = Rc::new(Gate {
            application: application.downgrade(),
            phase: Cell::new(Phase::Idle),
            pending: RefCell::new(VecDeque::new()),
            window: RefCell::new(None),
        });
        *slot = Some(gate.clone());
        application.connect_shutdown(|_| GATE.with_borrow_mut(|slot| *slot = None));
        gate
    });
    if gate.phase.get() == Phase::Ready {
        open(application, request);
        return;
    }
    // Bound reveal/open requests while a choice is pending. Collapse duplicate
    // plain activations, but retain file selections and their original order.
    let mut pending = gate.pending.borrow_mut();
    if pending.len() < 16 && (request.is_some() || !pending.iter().any(Option::is_none)) {
        pending.push_back(request);
    }
    drop(pending);
    if gate.phase.get() == Phase::Idle {
        gate.check();
    } else if let Some(window) = gate.window.borrow().as_ref() {
        window.present();
    }
}

fn open(application: &gtk::Application, request: Option<crate::file_manager::Request>) {
    match request {
        Some(request) => super::window::present_reveal(application, request),
        None => super::window::present(application),
    }
}

impl Gate {
    fn check(self: &Rc<Self>) {
        let Some(application) = self.application.upgrade() else {
            return;
        };
        self.phase.set(Phase::Checking);
        let gate = self.clone();
        let hold = application.hold();
        glib::MainContext::default().spawn_local(async move {
            let result = gio::spawn_blocking(|| Roots::current().inspect()).await;
            match result {
                Ok(Ok(Status::Ready)) => gate.ready(),
                Ok(Ok(Status::Consent { recovering })) => gate.prompt(if recovering {
                    "A previous Strata import did not finish. Resume from the local recovery record and backups, or continue with the Hermes data already present."
                } else {
                    "Import Strata settings, themes, playlists, ratings, saved searches and session data? Originals stay untouched. A private local backup is made before each copy. Existing Hermes files are kept. Caches and desktop integration settings are excluded."
                }),
                Ok(Err(error)) => gate.prompt(error.message()),
                Err(_) => gate.prompt("The import check stopped unexpectedly. Retry before continuing."),
            }
            drop(hold);
        });
    }

    fn choose(self: &Rc<Self>, choice: Choice) {
        let Some(application) = self.application.upgrade() else {
            return;
        };
        if self.phase.get().busy() || self.phase.get() == Phase::Ready {
            return;
        }
        self.phase.set(Phase::Working);
        if let Some(window) = self.window.borrow().as_ref() {
            window.set_sensitive(false);
        }
        let gate = self.clone();
        let hold = application.hold();
        glib::MainContext::default().spawn_local(async move {
            let result = gio::spawn_blocking(move || Roots::current().decide(choice)).await;
            match result {
                Ok(Ok(())) => gate.ready(),
                Ok(Err(error)) => gate.prompt(error.message()),
                Err(_) => gate.prompt(
                    "Import stopped unexpectedly. Retry to resume from the local recovery record.",
                ),
            }
            drop(hold);
        });
    }

    fn ready(self: &Rc<Self>) {
        let Some(application) = self.application.upgrade() else {
            return;
        };
        if self.phase.replace(Phase::Ready) == Phase::Ready {
            return;
        }
        // No controller, ThemeManager, Trails, collections, or browser window
        // exists until this point. There is no legacy-store fallback.
        super::initialize_stores(&application);
        let pending = self.pending.take();
        for request in pending {
            open(&application, request);
        }
        if let Some(window) = self.window.take() {
            window.destroy();
        }
    }

    fn prompt(self: &Rc<Self>, message: &str) {
        let Some(application) = self.application.upgrade() else {
            return;
        };
        self.phase.set(Phase::Waiting);
        if let Some(window) = self.window.take() {
            window.destroy();
        }
        // Use native GTK styling; do not construct ThemeManager before consent.
        let window = gtk::ApplicationWindow::builder()
            .application(&application)
            .title("Set up Hermes")
            .default_width(520)
            .resizable(false)
            .build();
        let content = gtk::Box::new(gtk::Orientation::Vertical, 18);
        content.set_margin_top(24);
        content.set_margin_bottom(24);
        content.set_margin_start(24);
        content.set_margin_end(24);
        let title = gtk::Label::new(Some("Choose your Hermes data"));
        title.add_css_class("title-2");
        title.set_xalign(0.0);
        content.append(&title);
        let description = gtk::Label::new(Some(message));
        description.set_wrap(true);
        description.set_max_width_chars(64);
        description.set_xalign(0.0);
        content.append(&description);
        for (label, choice) in [
            ("Import / resume Strata data", Some(Choice::Import)),
            ("Continue with Hermes data", Some(Choice::KeepHermes)),
            ("Retry check", None),
        ] {
            let button = gtk::Button::with_label(label);
            let weak = Rc::downgrade(self);
            button.connect_clicked(move |_| {
                if let Some(gate) = weak.upgrade() {
                    match choice {
                        Some(choice) => gate.choose(choice),
                        None => {
                            if let Some(window) = gate.window.borrow().as_ref() {
                                window.set_sensitive(false);
                            }
                            gate.check();
                        }
                    }
                }
            });
            content.append(&button);
        }
        let weak = Rc::downgrade(self);
        window.connect_close_request(move |_| {
            if let Some(gate) = weak.upgrade() {
                if gate.phase.get().busy() {
                    return glib::Propagation::Stop;
                }
                if let Some(application) = gate.application.upgrade() {
                    application.quit();
                }
            }
            glib::Propagation::Proceed
        });
        window.set_child(Some(&content));
        window.present();
        self.window.replace(Some(window));
    }
}

#[cfg(test)]
mod tests;
