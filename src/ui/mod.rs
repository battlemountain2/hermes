// SPDX-License-Identifier: GPL-3.0-or-later

mod blur;
mod browser;
mod browser_modes;
pub mod folder_tree;
mod motion;
mod music;
mod seek;
pub mod preview;
pub mod recent_folders;
pub mod search;
mod settings;
mod theme;
pub mod thumbnail;
mod trails;
pub mod status_bar;
pub mod table_view;
mod window;

pub use window::{present, present_location};

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
