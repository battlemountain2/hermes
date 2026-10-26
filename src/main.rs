// SPDX-License-Identifier: GPL-3.0-or-later

mod adapters;
mod app;
mod assets;
mod build_info;
mod file_manager;
mod identity;
mod metrics;
mod model;
mod mpris;
mod music;
mod playback;
mod portal;
mod sandbox;
mod sandbox_helper;
mod services;
mod ui;

use gtk::{gio, prelude::*};

fn main() -> gtk::glib::ExitCode {
    let arguments: Vec<_> = std::env::args().collect();
    if arguments
        .get(1)
        .is_some_and(|value| value == "--desktop-integration-info")
    {
        println!(r#"{{"capability":"HERMES_FILE_MANAGER1_V1","picker_capability":"HERMES_FILE_CHOOSER_V1","folders":true,"picker":true}}"#);
        return gtk::glib::ExitCode::SUCCESS;
    }
    if arguments.get(1).is_some_and(|value| value == "--portal-file-chooser") {
        return portal::run();
    }
    if arguments.get(1).is_some_and(|value| value == "--playback-worker") {
        if let Err(error) = playback::worker::run() {
            playback::worker::report_error(&error);
            eprintln!("Playback worker failed: {error}");
            return gtk::glib::ExitCode::FAILURE;
        }
        return gtk::glib::ExitCode::SUCCESS;
    }
    if arguments
        .get(1)
        .is_some_and(|value| value == "--preview-helper")
    {
        if let Err(error) = sandbox_helper::run(&arguments[2..]) {
            eprintln!("Preview helper failed: {error}");
            return gtk::glib::ExitCode::FAILURE;
        }
        return gtk::glib::ExitCode::SUCCESS;
    }

    metrics::initialize();
    if let Err(error) = tracing_subscriber::fmt::try_init() {
        eprintln!("Unable to initialize logging: {error}");
    }

    if let Err(error) = assets::prepare() {
        eprintln!("Unable to prepare bundled assets: {error}");
    }

    gtk::glib::set_application_name(identity::NAME);
    let application_id = identity::application_id(
        std::env::var_os("HERMES_DEV").is_some() || std::env::var_os("STRATA_DEV").is_some(),
    );
    let application = gtk::Application::builder()
        .application_id(application_id)
        .flags(gio::ApplicationFlags::HANDLES_OPEN)
        .build();

    application.connect_startup(|_| gtk::Window::set_default_icon_name(identity::ICON_NAME));
    application.connect_startup(file_manager::initialize);
    application.connect_startup(ui::initialize_music);
    application.connect_startup(ui::initialize_saved_searches);
    application.connect_startup(ui::initialize_ratings);
    application.connect_activate(ui::present);
    application.connect_open(|application, files, _| {
        for file in files.iter().take(16) {
            ui::present_reveal(
                application,
                file_manager::Request {
                    directory: file
                        .path()
                        .map(crate::model::Location::local)
                        .unwrap_or_else(|| crate::model::Location::uri(file.uri())),
                    selection: Vec::new(),
                    properties: false,
                    startup_id: String::new(),
                },
            );
        }
    });
    application.run()
}
