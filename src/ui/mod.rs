// SPDX-License-Identifier: GPL-3.0-or-later

mod audio_preferences;
mod blur;
mod browser;
mod browser_modes;
mod context_menu;
pub mod folder_tree;
mod formatting;
pub mod home_dashboard;
mod hover;
mod legacy_import;
mod motion;
mod music;
pub(crate) mod picker;
pub mod preview;
pub mod quick_look;
pub mod recent_folders;
mod saved_searches;
pub mod search;
mod seek;
mod settings;
pub mod shortcuts;
pub mod split_view;
pub mod status_bar;
pub mod table_view;
pub(crate) mod theme;
pub mod thumbnail;
mod trails;
mod window;

pub fn present(application: &gtk::Application) {
    legacy_import::present(application, None);
}

pub(crate) fn present_reveal(
    application: &gtk::Application,
    request: crate::file_manager::Request,
) {
    legacy_import::present(application, Some(request));
}

fn initialize_stores(application: &gtk::Application) {
    initialize_music(application);
    initialize_saved_searches(application);
    initialize_ratings(application);
}

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
