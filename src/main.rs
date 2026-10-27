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
mod paths;
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
        .iter()
        .any(|value| value == "--version" || value == "-v" || value == "-V")
    {
        println!("{} {} ({})", identity::NAME, build_info::VERSION, build_info::COMMIT);
        return gtk::glib::ExitCode::SUCCESS;
    }
    if arguments
        .iter()
        .any(|value| value == "--diagnostics" || value == "--info")
    {
        print_diagnostics();
        return gtk::glib::ExitCode::SUCCESS;
    }
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

    let _ = paths::migrate_legacy_data_if_needed();

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
            let is_dir = file.query_file_type(
                gio::FileQueryInfoFlags::NOFOLLOW_SYMLINKS,
                None::<&gio::Cancellable>,
            ) == gio::FileType::Directory;
            let (directory, selection) = if is_dir {
                (
                    file.path()
                        .map(crate::model::Location::local)
                        .unwrap_or_else(|| crate::model::Location::uri(file.uri())),
                    Vec::new(),
                )
            } else if let Some(parent) = file.parent() {
                let target = file
                    .path()
                    .map(crate::model::Location::local)
                    .unwrap_or_else(|| crate::model::Location::uri(file.uri()));
                (
                    parent
                        .path()
                        .map(crate::model::Location::local)
                        .unwrap_or_else(|| crate::model::Location::uri(parent.uri())),
                    vec![target],
                )
            } else {
                (
                    file.path()
                        .map(crate::model::Location::local)
                        .unwrap_or_else(|| crate::model::Location::uri(file.uri())),
                    Vec::new(),
                )
            };
            ui::present_reveal(
                application,
                file_manager::Request {
                    directory,
                    selection,
                    properties: false,
                    startup_id: String::new(),
                },
            );
        }
    });

    let sanitized_arguments: Vec<String> = arguments
        .into_iter()
        .filter(|arg| arg != "--reveal" && arg != "--select")
        .collect();
    application.run_with_args(&sanitized_arguments)
}

fn check_executable(binary: &str) -> bool {
    if let Some(paths) = std::env::var_os("PATH") {
        for path in std::env::split_paths(&paths) {
            let full = path.join(binary);
            if full.is_file() {
                return true;
            }
        }
    }
    false
}

fn print_diagnostics() {
    println!("=== {} Diagnostics ===", identity::NAME);
    println!("Version: {} ({})", build_info::VERSION, build_info::COMMIT);
    println!(
        "Build Profile: {}",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
    );
    println!("Target: {}-{}", std::env::consts::OS, std::env::consts::ARCH);
    println!(
        "Session Type: {}",
        std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| "unknown".into())
    );
    println!(
        "Desktop: {}",
        std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_else(|_| "unknown".into())
    );
    println!(
        "GTK Version: {}.{}.{}",
        gtk::major_version(),
        gtk::minor_version(),
        gtk::micro_version()
    );
    println!("Config Directory: {}", paths::config_dir().display());
    println!("Data Directory: {}", paths::data_dir().display());
    println!("Cache Directory: {}", paths::cache_dir().display());
    println!("State Directory: {}", paths::state_dir().display());

    println!("\nPreview Helpers:");
    for (name, binary) in [
        ("Bubblewrap (sandbox)", "bwrap"),
        ("FFmpeg (media playback)", "ffmpeg"),
        ("FFmpeg Thumbnailer (video covers)", "ffmpegthumbnailer"),
        ("ImageMagick (scaled images)", "magick"),
        ("ImageMagick convert (fallback)", "convert"),
        ("LibreOffice (document layout)", "soffice"),
    ] {
        let available = check_executable(binary);
        println!("  - {name}: {}", if available { "available" } else { "not found" });
    }
}
