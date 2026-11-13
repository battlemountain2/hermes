use super::*;
use crate::sandbox::browser::format_tests::FixtureDirectory;
use std::{
    cell::RefCell,
    fs,
    os::unix::{ffi::OsStringExt, fs::symlink},
};

#[test]
#[ignore = "Private GTK display and isolated XDG roots; creates only generated folders"]
fn private_new_folder_control_creates_and_enters_folder_without_changing_save_name() {
    glib::set_prgname(Some("hermes-picker-test"));
    gtk::init().expect("initialize the isolated GTK display");
    crate::assets::prepare().expect("register embedded test assets");
    let theme = prepare();
    theme.select_theme("graphite");
    let application = gtk::Application::builder()
        .application_id("io.github.battlemountain2.Hermes.FolderTest")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    application
        .register(None::<&gio::Cancellable>)
        .expect("expected test value from application.register(None::<&gio::Cancellable>)");
    let fixture = FixtureDirectory::new();
    let request = Request::parse(
        "SaveFile",
        "Save generated archive".into(),
        String::new(),
        Options::from([
            (
                "current_folder".into(),
                [fixture.0.as_os_str().as_encoded_bytes(), &[0]]
                    .concat()
                    .to_variant(),
            ),
            ("current_name".into(), "download.7z".to_variant()),
        ]),
    )
    .expect("expected test value from Request::parse( \"SaveFile\", \"Save generated archive\".into(), String::new(), Options::from([ ( \"current_folder\".into(), [fixture...");
    let picker = Picker::new(&application, request, Rc::new(|_| {}));
    until(|| {
        picker
            .0
            .dialog
            .current_folder()
            .and_then(|folder| folder.path())
            .as_ref()
            == Some(&fixture.0)
    });
    fn find<T: IsA<gtk::Widget> + glib::object::IsClass>(
        widget: &gtk::Widget,
        predicate: &impl Fn(&T) -> bool,
    ) -> Option<T> {
        if let Ok(found) = widget.clone().downcast::<T>()
            && predicate(&found)
        {
            return Some(found);
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            child = current.next_sibling();
            if let Some(found) = find(&current, predicate) {
                return Some(found);
            }
        }
        None
    }
    let button = find::<gtk::MenuButton>(picker.0.dialog.upcast_ref(), &|button| {
        button.has_css_class("picker-new-folder")
    })
    .expect("expected test value from find::<gtk::MenuButton>(picker.0.dialog.upcast_ref(), &|button| { button.has_css_class(\"picker-new-folder\") })");
    button.popup();
    let popover = button
        .popover()
        .expect("expected test value from button.popover()");
    let field = find::<gtk::Entry>(popover.upcast_ref(), &|entry| {
        entry.placeholder_text().as_deref() == Some("Folder name")
    })
    .expect("expected test value from find::<gtk::Entry>(popover.upcast_ref(), &|entry| { entry.placeholder_text().as_deref() == Some(\"Folder name\") })");
    let create = find::<gtk::Button>(popover.upcast_ref(), &|button| {
        button.label().as_deref() == Some("Create folder")
    })
    .expect("expected test value from find::<gtk::Button>(popover.upcast_ref(), &|button| { button.label().as_deref() == Some(\"Create folder\") })");
    assert!(!create.is_sensitive());
    field.set_text("../outside");
    assert!(!create.is_sensitive());
    field.set_text("New folder");
    assert!(create.is_sensitive());
    create.emit_clicked();
    until(|| {
        picker
            .0
            .dialog
            .current_folder()
            .and_then(|folder| folder.path())
            .as_ref()
            == Some(&fixture.0.join("New folder"))
    });
    assert_eq!(
        picker.0.dialog.current_name().as_deref(),
        Some("download.7z")
    );
    assert!(fixture.0.join("New folder").is_dir());
    picker.close();
}

#[test]
fn chooser_styles_use_only_existing_semantic_color_tokens() {
    let styles = include_str!("../../style.css");
    let chooser = styles
        .split(".hermes-picker,")
        .nth(1)
        .expect("expected test value from styles.split(\".hermes-picker,\").nth(1)");
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
            .expect("expected test value from value .split(|character: char| !character.is_ascii_alphanumeric() && character != '_') .next()");
        assert!(
            known.contains(&token),
            "Undefined chooser color token: {token}"
        );
    }
}

#[test]
fn chooser_typography_inherits_the_application_font_and_scale() {
    let styles = include_str!("../../style.css");
    let chooser = styles
        .split(".hermes-picker,")
        .nth(1)
        .expect("expected test value from styles.split(\".hermes-picker,\").nth(1)");
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
    fs::write(&file, b"original").expect("write generated fixture");
    let alias = fixture.0.join("folder alias");
    symlink(&fixture.0, &alias).expect("expected test value from symlink(&fixture.0, &alias)");
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
                    .expect("expected test value from selected_paths(&open, vec![gio::File::for_path(&file)]) .await");
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
                    .expect("expected test value from selected_paths( &Kind::Open { directory: false, multiple: true }, vec![gio::File::for_path(&file); 2] ) .await")
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
                assert_eq!(fs::read(&file).expect("read generated fixture"), b"original");
            })
        })
        .expect("read generated fixture");
}

#[test]
fn save_files_avoids_existing_broken_symlinks_and_same_request_name_collisions() {
    let fixture = FixtureDirectory::new();
    fs::write(fixture.0.join("report.txt"), b"keep").expect("write generated fixture");
    symlink(fixture.0.join("missing"), fixture.0.join("report (1).txt")).expect("expected test value from symlink(fixture.0.join(\"missing\"), fixture.0.join(\"report (1).txt\"))");
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
                .expect("expected test value from selected_paths( &Kind::SaveFiles(names), vec![gio::File::for_path(&fixture.0)], ) .await");
                assert_eq!(uris.len(), 3);
                assert!(uris[0].ends_with("report%20(2).txt"));
                assert!(uris[1].ends_with("report%20(3).txt"));
                assert!(uris[2].ends_with("native%FF.txt"));
                assert_eq!(fs::read_dir(&fixture.0).expect("expected test value from fs::read_dir(&fixture.0)").count(), 2);
            })
        })
        .expect("expected test value from context .with_thread_default(|| { context.block_on(async { let uris = selected_paths( &Kind::SaveFiles(names), vec![gio::File::...");
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
    glib::set_prgname(Some("hermes-picker-test"));
    gtk::init().expect("initialize the isolated GTK display");
    glib::set_application_name(crate::identity::NAME);
    crate::assets::prepare().expect("register embedded test assets");
    let theme = prepare();
    let application = gtk::Application::builder()
        .application_id("io.github.battlemountain2.Hermes.PickerTest")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    application
        .register(None::<&gio::Cancellable>)
        .expect("expected test value from application.register(None::<&gio::Cancellable>)");
    let fixture = FixtureDirectory::new();
    let target = fixture.0.join("chosen.txt");
    fs::write(&target, b"generated").expect("write generated fixture");
    let _ = fs::create_dir(fixture.0.join("Projects"));
    let _ = fs::write(fixture.0.join("music.flac"), b"audio");
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
    .expect("expected test value from Request::parse( \"OpenFile\", \"Choose generated file\".into(), String::new(), Options::from([ ( \"current_folder\".into(), [fixture....");
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
        .expect(
            "expected test value from picker .0 .dialog .set_file(&gio::File::for_path(&target))",
        );
    until(|| picker.0.dialog.file().and_then(|file| file.path()).as_ref() == Some(&target));
    let accept = picker
        .0
        .dialog
        .widget_for_response(gtk::ResponseType::Accept)
        .expect("expected test value from picker .0 .dialog .widget_for_response(gtk::ResponseType::Accept)");
    fn check_decorations(w: &gtk::Widget, thumbnails: &Cell<usize>, file_icons: &Cell<usize>) {
        if w.type_().name() == "GtkFileThumbnail" && w.has_css_class("picker-thumbnail-connected") {
            thumbnails.set(thumbnails.get() + 1);
        }
        if w.has_css_class("picker-file-icon") {
            file_icons.set(file_icons.get() + 1);
        }
        let mut child = w.first_child();
        while let Some(c) = child {
            child = c.next_sibling();
            check_decorations(&c, thumbnails, file_icons);
        }
    }
    let thumbnails = Cell::new(0);
    let file_icons = Cell::new(0);
    until(|| {
        thumbnails.set(0);
        file_icons.set(0);
        check_decorations(picker.0.dialog.upcast_ref(), &thumbnails, &file_icons);
        thumbnails.get() > 0
    });
    assert!(
        thumbnails.get() > 0,
        "Expected decorated GtkFileThumbnail widgets"
    );
    assert!(
        file_icons.get() > 0,
        "Expected GtkImage with picker-file-icon"
    );
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
                .expect("expected test value from accept .style_context() .lookup_color(\"theme_on_accent\")")
        );
    }
    if let Some(path) = std::env::var_os("HERMES_PICKER_SCREENSHOT") {
        // Broadway has no browser client to acknowledge frames in this test.
        // Use a fresh chooser after the live-theme assertions so its render
        // nodes do not depend on a previous, unacknowledged theme frame.
        theme.select_theme("graphite");
        let sample = Picker::new(&application, request.clone(), Rc::new(|_| {}));
        until(|| sample.0.dialog.width() > 0);
        let deadline = Instant::now() + Duration::from_millis(250);
        while Instant::now() < deadline {
            while glib::MainContext::default().iteration(false) {}
            std::thread::sleep(Duration::from_millis(5));
        }
        fn invalidate(widget: &gtk::Widget) {
            widget.queue_draw();
            let mut child = widget.first_child();
            while let Some(current) = child {
                child = current.next_sibling();
                invalidate(&current);
            }
        }
        invalidate(sample.0.dialog.upcast_ref());
        sample
            .0
            .dialog
            .allocate(sample.0.dialog.width(), sample.0.dialog.height(), -1, None);
        let snapshot = gtk::Snapshot::new();
        let mut child = sample.0.dialog.first_child();
        while let Some(current) = child {
            child = current.next_sibling();
            sample.0.dialog.snapshot_child(&current, &snapshot);
        }
        let node = snapshot
            .to_node()
            .expect("The chooser must render before saving its screenshot");
        let renderer = gtk::gsk::CairoRenderer::new();
        renderer
            .realize(None::<&gtk::gdk::Surface>)
            .expect("expected test value from renderer.realize(None::<&gtk::gdk::Surface>)");
        renderer
            .render_texture(&node, None)
            .save_to_png(path)
            .expect(
                "expected test value from renderer .render_texture(&node, None) .save_to_png(path)",
            );
        renderer.unrealize();
        sample.close();
    }
    picker.0.dialog.response(gtk::ResponseType::Accept);
    until(|| !received.borrow().is_empty());
    let results = received.borrow()[0]
        .clone()
        .expect("expected test value from received.borrow()[0].clone()");
    assert_eq!(
        results["uris"]
            .get::<Vec<String>>()
            .expect("expected test value from results[\"uris\"].get::<Vec<String>>()"),
        [gio::File::for_path(&target).uri().to_string()]
    );
    assert_eq!(
        results["current_filter"]
            .get::<crate::portal::request::Filter>()
            .expect("expected test value from results[\"current_filter\"] .get::<crate::portal::request::Filter>()"),
        filter
    );
    assert_eq!(
        results["choices"]
            .get::<Vec<(String, String)>>()
            .expect("expected test value from results[\"choices\"].get::<Vec<(String, String)>>()"),
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
    glib::set_prgname(Some("hermes-picker-test"));
    gtk::init().expect("initialize the isolated GTK display");
    glib::set_application_name(crate::identity::NAME);
    crate::assets::prepare().expect("register embedded test assets");
    let _theme = prepare();
    let application = gtk::Application::builder()
        .application_id("io.github.battlemountain2.Hermes.SaveTest")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    application
        .register(None::<&gio::Cancellable>)
        .expect("expected test value from application.register(None::<&gio::Cancellable>)");
    let fixture = FixtureDirectory::new();
    let existing = fixture.0.join("existing.txt");
    fs::write(&existing, b"original").expect("write generated fixture");
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
    .expect("expected test value from Request::parse( \"SaveFile\", \"Save generated file\".into(), String::new(), options, )");
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
    let results = received.borrow()[0]
        .clone()
        .expect("expected test value from received.borrow()[0].clone()");
    assert_eq!(
        results["uris"]
            .get::<Vec<String>>()
            .expect("expected test value from results[\"uris\"].get::<Vec<String>>()"),
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
    let child = confirmation().expect("expected test value from confirmation()");
    picker.close();
    until(|| !child.is_visible());
    assert_eq!(blocked.get(), 0);
    assert_eq!(
        fs::read(&existing).expect("read generated fixture"),
        b"original"
    );
    assert!(application.windows().is_empty());
}
