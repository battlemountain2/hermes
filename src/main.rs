// SPDX-License-Identifier: GPL-3.0-or-later

mod adapters;
mod app;
mod assets;
mod build_info;
mod metrics;
mod model;
mod mpris;
mod music;
mod playback;
mod sandbox;
mod sandbox_helper;
mod services;
mod ui;

use gtk::{gio, prelude::*};

const APPLICATION_ID: &str = "io.github.lgse.Strata";

fn main() -> gtk::glib::ExitCode {
    let arguments: Vec<_> = std::env::args().collect();
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

    let application_id = if std::env::var_os("STRATA_DEV").is_some() {
        "io.github.lgse.Strata.Dev"
    } else {
        APPLICATION_ID
    };
    let application = gtk::Application::builder()
        .application_id(application_id)
        .flags(gio::ApplicationFlags::HANDLES_OPEN)
        .build();

    application.connect_startup(ui::initialize_music);
    application.connect_activate(ui::present);
    application.connect_open(|application, files, _| {
        let location = files.first().and_then(gio::File::path);
        ui::present_location(application, location);
    });
    application.run()
}
