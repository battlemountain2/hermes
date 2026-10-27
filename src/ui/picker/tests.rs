use super::*;
use crate::sandbox::browser::format_tests::FixtureDirectory;
use std::{
    cell::RefCell,
    fs,
    os::unix::{ffi::OsStringExt, fs::symlink},
};

#[test]
fn chooser_styles_use_only_existing_semantic_color_tokens() {
    let styles = include_str!("../../style.css");
    let chooser = styles.split(".hermes-picker,").nth(1).unwrap();
    let known = [
        "theme_bg",
        "theme_surface",
        "theme_text",
        "theme_accent",
        "theme_danger",
        "theme_muted",
        "theme_highlight",
        "theme_border",
        "theme_dim_text",
        "theme_on_accent",
    ];
    for value in chooser.split('@').skip(1) {
        let token = value
            .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
            .next()
            .unwrap();
        assert!(
            known.contains(&token),
            "Undefined chooser color token: {token}"
        );
    }
}

#[test]
fn chooser_typography_inherits_the_application_font_and_scale() {
    let styles = include_str!("../../style.css");
    let chooser = styles.split(".hermes-picker,").nth(1).unwrap();
    assert!(!chooser.contains("Noto Sans"));
    assert!(!chooser.contains("font-size: 14px"));
    assert!(chooser.contains("font-family: inherit"));
    assert!(chooser.contains("font-size: inherit"));
}

#[test]
fn conflict_names_keep_extensions_hidden_names_and_native_bytes() {
    for (name, expected) in [
        ("report.pdf", "report (1).pdf"),
        ("archive.tar.gz", "archive.tar (1).gz"),
        (".hidden", ".hidden (1)"),
        ("folder", "folder (1)"),
    ] {
        assert_eq!(numbered(Path::new(name).as_os_str(), 0), name);
        assert_eq!(numbered(Path::new(name).as_os_str(), 1), expected);
    }
    assert_eq!(
        numbered(&OsString::from_vec(b"byte\xff.txt".to_vec()), 2).as_encoded_bytes(),
        b"byte\xff (2).txt"
    );
}

#[test]
fn open_save_and_multiple_selection_validate_native_paths_without_writing_files() {
    let fixture = FixtureDirectory::new();
    let file = fixture.0.join("generated photo.txt");
    fs::write(&file, b"original").unwrap();
    let alias = fixture.0.join("folder alias");
    symlink(&fixture.0, &alias).unwrap();
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            context.block_on(async {
                let open = Kind::Open {
                    directory: false,
                    multiple: false,
                };
                let selected = selected_paths(&open, vec![gio::File::for_path(&file)])
                    .await
                    .unwrap();
                assert_eq!(selected, [gio::File::for_path(&file).uri().to_string()]);
                assert!(
                    selected_paths(&open, vec![gio::File::for_path(&fixture.0)])
                        .await
                        .is_err()
                );
                assert!(
                    selected_paths(&open, vec![gio::File::for_path(fixture.0.join("missing"))])
                        .await
                        .is_err()
                );
                assert!(
                    selected_paths(&open, vec![gio::File::for_uri("sftp://host/file")])
                        .await
                        .is_err()
                );
                assert!(selected_paths(&open, vec![]).await.is_err());
                assert!(
                    selected_paths(&open, vec![gio::File::for_path(&file); 2])
                        .await
                        .is_err()
                );
                assert_eq!(
                    selected_paths(
                        &Kind::Open {
                            directory: false,
                            multiple: true
                        },
                        vec![gio::File::for_path(&file); 2]
                    )
                    .await
                    .unwrap()
                    .len(),
                    2
                );
                assert!(
                    selected_paths(
                        &Kind::Open {
                            directory: true,
                            multiple: false
                        },
                        vec![gio::File::for_path(&alias)]
                    )
                    .await
                    .is_ok()
                );
                let destination = fixture.0.join("new.txt");
                assert!(
                    selected_paths(&Kind::Save, vec![gio::File::for_path(&destination)])
                        .await
                        .is_ok()
                );
                assert!(
                    selected_paths(&Kind::Save, vec![gio::File::for_path(&fixture.0)])
                        .await
                        .is_err()
                );
                assert!(!destination.exists());
                assert_eq!(fs::read(&file).unwrap(), b"original");
            })
        })
        .unwrap();
}

#[test]
fn save_files_avoids_existing_broken_symlinks_and_same_request_name_collisions() {
    let fixture = FixtureDirectory::new();
    fs::write(fixture.0.join("report.txt"), b"keep").unwrap();
    symlink(fixture.0.join("missing"), fixture.0.join("report (1).txt")).unwrap();
    let names = vec![
        "report.txt".into(),
        "report.txt".into(),
        OsString::from_vec(b"native\xff.txt".to_vec()),
    ];
    let context = glib::MainContext::new();
    context
        .with_thread_default(|| {
            context.block_on(async {
                let uris = selected_paths(
                    &Kind::SaveFiles(names),
                    vec![gio::File::for_path(&fixture.0)],
                )
                .await
                .unwrap();
                assert_eq!(uris.len(), 3);
                assert!(uris[0].ends_with("report%20(2).txt"));
                assert!(uris[1].ends_with("report%20(3).txt"));
                assert!(uris[2].ends_with("native%FF.txt"));
                assert_eq!(fs::read_dir(&fixture.0).unwrap().count(), 2);
            })
        })
        .unwrap();
}

#[track_caller]
fn until(condition: impl Fn() -> bool) {
    let started = Instant::now();
    let context = glib::MainContext::default();
    while !condition() {
        assert!(
            started.elapsed() < Duration::from_secs(8),
            "Timed out waiting for private chooser"
        );
        context.iteration(false);
        std::thread::sleep(Duration::from_millis(3));
    }
}

#[test]
#[ignore = "Private GTK display and isolated XDG roots only; generated chooser files, no user documents"]
fn private_chooser_returns_file_filters_choices_and_cancels_exactly_once() {
    gtk::init().unwrap();
    glib::set_application_name(crate::identity::NAME);
    crate::assets::prepare().unwrap();
    let theme = prepare();
    let application = gtk::Application::builder()
        .application_id("io.github.battlemountain2.Hermes.PickerTest")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    application.register(None::<&gio::Cancellable>).unwrap();
    let fixture = FixtureDirectory::new();
    let target = fixture.0.join("chosen.txt");
    fs::write(&target, b"generated").unwrap();
    let filter: crate::portal::request::Filter = ("Text".into(), vec![(0, "*.txt".into())]);
    let choices: Vec<crate::portal::request::Choice> =
        vec![("readonly".into(), "Read only".into(), vec![], "true".into())];
    let request = Request::parse(
        "OpenFile",
        "Choose generated file".into(),
        String::new(),
        Options::from([
            (
                "current_folder".into(),
                [fixture.0.as_os_str().as_encoded_bytes(), &[0]]
                    .concat()
                    .to_variant(),
            ),
            ("filters".into(), vec![filter.clone()].to_variant()),
            ("choices".into(), choices.to_variant()),
        ]),
    )
    .unwrap();
    let received = Rc::new(RefCell::new(Vec::new()));
    let captured = received.clone();
    let picker = Picker::new(
        &application,
        request.clone(),
        Rc::new(move |result| captured.borrow_mut().push(result)),
    );
    until(|| {
        picker
            .0
            .dialog
            .current_folder()
            .and_then(|folder| folder.path())
            .as_ref()
            == Some(&fixture.0)
    });
    picker
        .0
        .dialog
        .set_file(&gio::File::for_path(&target))
        .unwrap();
    until(|| picker.0.dialog.file().and_then(|file| file.path()).as_ref() == Some(&target));
    let accept = picker
        .0
        .dialog
        .widget_for_response(gtk::ResponseType::Accept)
        .unwrap();
    for id in ["paper", "graphite", "azure-glow"] {
        theme.select_theme(id);
        let deadline = Instant::now() + Duration::from_millis(120);
        while Instant::now() < deadline {
            glib::MainContext::default().iteration(false);
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(
            accept.color(),
            accept
                .style_context()
                .lookup_color("theme_on_accent")
                .unwrap()
        );
    }
    if let Some(path) = std::env::var_os("HERMES_PICKER_SCREENSHOT") {
        until(|| picker.0.dialog.width() > 0);
        let paintable = gtk::WidgetPaintable::new(Some(&picker.0.dialog));
        let snapshot = gtk::Snapshot::new();
        paintable.snapshot(
            &snapshot,
            f64::from(picker.0.dialog.width()),
            f64::from(picker.0.dialog.height()),
        );
        if let Some(node) = snapshot.to_node() {
            let renderer = gtk::gsk::CairoRenderer::new();
            renderer.realize(None::<&gtk::gdk::Surface>).unwrap();
            renderer
                .render_texture(&node, None)
                .save_to_png(path)
                .unwrap();
            renderer.unrealize();
        }
    }
    picker.0.dialog.response(gtk::ResponseType::Accept);
    until(|| !received.borrow().is_empty());
    let results = received.borrow()[0].clone().unwrap();
    assert_eq!(
        results["uris"].get::<Vec<String>>().unwrap(),
        [gio::File::for_path(&target).uri().to_string()]
    );
    assert_eq!(
        results["current_filter"]
            .get::<crate::portal::request::Filter>()
            .unwrap(),
        filter
    );
    assert_eq!(
        results["choices"].get::<Vec<(String, String)>>().unwrap(),
        [("readonly".into(), "true".into())]
    );
    picker.0.dialog.response(gtk::ResponseType::Cancel);
    picker.close();
    assert_eq!(received.borrow().len(), 1);
    let canceled = Rc::new(Cell::new(0));
    let captured = canceled.clone();
    let picker = Picker::new(
        &application,
        request,
        Rc::new(move |result| {
            assert!(result.is_none());
            captured.set(captured.get() + 1);
        }),
    );
    picker.0.dialog.response(gtk::ResponseType::Cancel);
    picker.close();
    assert_eq!(canceled.get(), 1);
    assert!(application.windows().is_empty());
}

#[test]
#[ignore = "Private GTK display and generated save destinations only; never writes to user documents"]
fn private_save_requires_overwrite_confirmation_and_closes_child_dialogs() {
    gtk::init().unwrap();
    glib::set_application_name(crate::identity::NAME);
    crate::assets::prepare().unwrap();
    let _theme = prepare();
    let application = gtk::Application::builder()
        .application_id("io.github.battlemountain2.Hermes.SaveTest")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    application.register(None::<&gio::Cancellable>).unwrap();
    let fixture = FixtureDirectory::new();
    let existing = fixture.0.join("existing.txt");
    fs::write(&existing, b"original").unwrap();
    let options = Options::from([
        (
            "current_folder".into(),
            [fixture.0.as_os_str().as_encoded_bytes(), &[0]]
                .concat()
                .to_variant(),
        ),
        ("current_name".into(), "new.txt".to_variant()),
    ]);
    let request = Request::parse(
        "SaveFile",
        "Save generated file".into(),
        String::new(),
        options,
    )
    .unwrap();
    let received = Rc::new(RefCell::new(Vec::new()));
    let captured = received.clone();
    let picker = Picker::new(
        &application,
        request.clone(),
        Rc::new(move |result| captured.borrow_mut().push(result)),
    );
    until(|| {
        picker
            .0
            .dialog
            .current_folder()
            .and_then(|folder| folder.path())
            .as_ref()
            == Some(&fixture.0)
    });
    picker.0.dialog.response(gtk::ResponseType::Accept);
    until(|| !received.borrow().is_empty());
    let results = received.borrow()[0].clone().unwrap();
    assert_eq!(
        results["uris"].get::<Vec<String>>().unwrap(),
        [gio::File::for_path(fixture.0.join("new.txt"))
            .uri()
            .to_string()]
    );
    assert!(!fixture.0.join("new.txt").exists());
    picker.close();
    let blocked = Rc::new(Cell::new(0));
    let captured = blocked.clone();
    let picker = Picker::new(
        &application,
        request,
        Rc::new(move |_| captured.set(captured.get() + 1)),
    );
    until(|| {
        picker
            .0
            .dialog
            .current_folder()
            .and_then(|folder| folder.path())
            .as_ref()
            == Some(&fixture.0)
    });
    picker.0.dialog.set_current_name("existing.txt");
    picker.0.dialog.response(gtk::ResponseType::Accept);
    let confirmation = || {
        gtk::Window::list_toplevels()
            .into_iter()
            .filter_map(|widget| widget.downcast::<gtk::Window>().ok())
            .find(|window| {
                window.is_visible()
                    && window.transient_for().as_ref() == Some(picker.0.dialog.upcast_ref())
            })
    };
    until(|| confirmation().is_some());
    assert_eq!(
        blocked.get(),
        0,
        "Existing destination was accepted without confirmation"
    );
    let child = confirmation().unwrap();
    picker.close();
    until(|| !child.is_visible());
    assert_eq!(blocked.get(), 0);
    assert_eq!(fs::read(&existing).unwrap(), b"original");
    assert!(application.windows().is_empty());
}
