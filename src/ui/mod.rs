// SPDX-License-Identifier: GPL-3.0-or-later

mod blur;
mod browser;
mod browser_modes;
mod context_menu;
pub mod folder_tree;
mod motion;
mod music;
mod seek;
pub mod preview;
pub mod recent_folders;
pub mod search;
mod saved_searches;
mod settings;
mod theme;
pub mod thumbnail;
mod trails;
pub mod status_bar;
pub mod table_view;
mod window;

pub use window::{present, present_location};

pub(crate) fn initialize_saved_searches(application: &gtk::Application) {
    use gtk::prelude::*;
    let store = crate::services::saved_searches::Store::shared();
    application.connect_shutdown(move |_| {
        if !store.flush(std::time::Duration::from_secs(1)) {
            tracing::warn!("Saved search changes did not finish cleanly before shutdown");
        }
    });
}

pub(crate) fn initialize_ratings(application: &gtk::Application) {
    use gtk::prelude::*;
    let store = crate::services::ratings::Store::shared();
    application.connect_shutdown(move |_| {
        if let Err(error) = store.flush(std::time::Duration::from_secs(1)) {
            tracing::warn!("Ratings changes did not finish cleanly before shutdown: {error}");
        }
    });
}

pub(crate) fn initialize_music(application: &gtk::Application) {
    use gtk::prelude::*;
    let controller = music::controller::Controller::for_application(application);
    let backend: std::rc::Rc<dyn crate::mpris::Backend> = std::rc::Rc::new(controller);
    let service = std::cell::RefCell::new(Some(crate::mpris::Service::start(application, backend)));
    application.connect_shutdown(move |_| {
        service.borrow_mut().take();
    });
}

#[cfg(test)]
pub(crate) fn test_music_controller() -> std::rc::Rc<music::controller::Controller> {
    music::controller::Controller::with_session(None)
}
