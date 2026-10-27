use super::*;

#[test]
fn entries_preserve_native_paths_and_file_sizes_without_reading_contents() {
    let file = gio::File::for_path("/tmp/generated photo.png");
    let info = gio::FileInfo::new();
    info.set_display_name("Generated photo.png");
    info.set_file_type(gio::FileType::Regular);
    info.set_size(123);
    let file_entry = entry(&file, &info).unwrap();
    assert_eq!(
        file_entry.location,
        Location::local("/tmp/generated photo.png")
    );
    assert_eq!(file_entry.size, MetadataValue::Known(123));
    assert_eq!(file_entry.display_name, "Generated photo.png");
    assert!(
        entry(
            &gio::File::for_uri("sftp://host/unmounted/photo.png"),
            &info
        )
        .is_none()
    );
}

#[test]
#[ignore = "Private GTK/XDG roots and built sandbox helper; generated PNG previews only"]
fn private_picker_renders_folder_and_selected_previews_without_stale_images() {
    use std::{fs, time::Instant};
    let root = std::path::PathBuf::from(std::env::var_os("HERMES_PICKER_TEST_ROOT").unwrap());
    assert_eq!(glib::user_cache_dir(), root.join("cache"));
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let theme = super::super::prepare();
    let directory = crate::sandbox::browser::format_tests::FixtureDirectory::new();
    for (name, color) in [
        ("first.png", 0xbe7044ff),
        ("second.png", 0x376f8aff),
        (".hidden.png", 0xffffff00),
    ] {
        let pixels =
            gtk::gdk_pixbuf::Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, true, 8, 320, 240)
                .unwrap();
        pixels.fill(color);
        fs::write(
            directory.0.join(name),
            pixels.save_to_bufferv("png", &[]).unwrap(),
        )
        .unwrap();
    }
    std::os::unix::fs::symlink(directory.0.join("first.png"), directory.0.join("alias.png"))
        .unwrap();
    fs::write(directory.0.join("notes.txt"), b"No image").unwrap();
    fs::create_dir(directory.0.join("Empty folder")).unwrap();
    gio::Settings::new("org.gtk.gtk4.Settings.FileChooser")
        .set_string("view-type", "grid")
        .unwrap();
    let application = gtk::Application::builder()
        .application_id("io.github.battlemountain2.Hermes.VisualsTest")
        .flags(gio::ApplicationFlags::NON_UNIQUE)
        .build();
    application.register(None::<&gio::Cancellable>).unwrap();
    let options = crate::portal::request::Options::from([(
        "current_folder".into(),
        [directory.0.as_os_str().as_encoded_bytes(), &[0]]
            .concat()
            .to_variant(),
    )]);
    let request = crate::portal::request::Request::parse(
        "OpenFile",
        "Choose a picture".into(),
        String::new(),
        options,
    )
    .unwrap();
    let picker = super::super::Picker::new(&application, request, Rc::new(|_| {}));
    let visuals = &picker.0.visuals;
    let wait_step = Cell::new(0);
    let wait = |condition: &dyn Fn() -> bool| {
        wait_step.set(wait_step.get() + 1);
        let deadline = Instant::now() + Duration::from_secs(15);
        while !condition() {
            if Instant::now() >= deadline {
                let mut tree = String::new();
                describe_widgets(picker.0.dialog.upcast_ref(), 0, &mut tree);
                fs::write(root.join("failure-widgets.txt"), tree).unwrap();
            }
            assert!(
                Instant::now() < deadline,
                "Preview did not finish at step {}: name={} visible={} selected={:?}",
                wait_step.get(),
                visuals.name.text(),
                visuals.widget.is_visible(),
                picker.0.dialog.file().map(|file| file.uri())
            );
            glib::MainContext::default().iteration(false);
            std::thread::sleep(Duration::from_millis(3));
        }
    };
    wait(&|| {
        picker
            .0
            .dialog
            .current_folder()
            .and_then(|folder| folder.path())
            .as_ref()
            == Some(&directory.0)
    });
    wait(&|| !find_file_images(picker.0.dialog.upcast_ref()).is_empty());
    assert!(!visuals.widget.is_visible());
    let idle = Visuals::new();
    idle.update(None, Some(gio::File::for_path(&directory.0)));
    assert!(idle.task.borrow().is_none());
    assert!(!idle.widget.is_visible());
    let native_icons = find_file_images(picker.0.dialog.upcast_ref());
    let folder_icon = native_icons
        .iter()
        .find(|image| {
            let parent = image.parent().unwrap();
            let property = if parent.find_property("file-info").is_some() {
                "file-info"
            } else {
                "info"
            };
            parent
                .property::<Option<gio::FileInfo>>(property)
                .is_some_and(|info| info.file_type() == gio::FileType::Directory)
        })
        .unwrap();
    folder_icon.set_icon_name(Some("folder"));
    assert!(folder_icon.icon_name().is_none());
    assert!(crate::assets::has_primary_icon(folder_icon));
    assert!(visuals.widget.root().is_some());
    let first = gio::File::for_path(directory.0.join("first.png"));
    let second = gio::File::for_path(directory.0.join("second.png"));
    picker.0.dialog.set_file(&first).unwrap();
    wait(&|| visuals.name.text() == "first.png" && visuals.widget.is_visible());
    let original_font = theme.interface_font().to_owned();
    let original_size = theme.interface_font_size();
    for font in ["jetbrains", "sans"] {
        theme.set_interface_font(font);
        theme.set_interface_font_size(15);
        wait(&|| {
            let name_font = visuals.name.pango_context().font_description().unwrap();
            let window_font = picker.0.dialog.pango_context().font_description().unwrap();
            name_font.family() == window_font.family()
                && name_font.size() == window_font.size()
                && name_font.size() == 15 * gtk::pango::SCALE
        });
    }
    theme.set_interface_font(&original_font);
    theme.set_interface_font_size(original_size);
    wait(&|| {
        visuals
            .name
            .pango_context()
            .font_description()
            .unwrap()
            .size()
            == original_size as i32 * gtk::pango::SCALE
    });
    picker.0.dialog.set_file(&first).unwrap();
    wait(&|| visuals.name.text() == "first.png");
    picker.0.dialog.set_file(&second).unwrap();
    wait(&|| visuals.name.text() == "second.png");
    visuals.update(Some(first), Some(gio::File::for_path(&directory.0)));
    visuals.update(
        Some(second.clone()),
        Some(gio::File::for_path(&directory.0)),
    );
    wait(&|| {
        visuals.name.text() == "second.png"
            && !crate::assets::has_primary_icon(&visuals.image)
            && visuals.image.paintable().is_some()
    });
    assert_eq!(visuals.image.pixel_size(), 256);
    assert_eq!(visuals.kind.text(), "PNG image");
    assert!(visuals.widget.is_visible());
    assert!(visuals.name.has_css_class("preview-title"));
    assert!(visuals.size.has_css_class("preview-metadata-value"));
    if let Some(path) = std::env::var_os("HERMES_PICKER_SCREENSHOT") {
        let mut tree = String::new();
        describe_widgets(picker.0.dialog.upcast_ref(), 0, &mut tree);
        fs::write(
            std::path::PathBuf::from(&path).with_file_name("picker-widgets.txt"),
            tree,
        )
        .unwrap();
        snapshot_picker(
            &picker.0.dialog,
            &theme.applied_tokens().unwrap().background,
            &std::path::PathBuf::from(path).with_file_name("visual-picker.png"),
        );
    }
    let filter = gtk::FileFilter::new();
    filter.add_pattern("first.png");
    picker.0.dialog.add_filter(&filter);
    picker.0.dialog.set_filter(&filter);
    let other_directory = crate::sandbox::browser::format_tests::FixtureDirectory::new();
    fs::copy(
        directory.0.join("first.png"),
        other_directory.0.join("first.png"),
    )
    .unwrap();
    let other_folder = gio::File::for_path(&other_directory.0);
    picker
        .0
        .dialog
        .set_current_folder(Some(&other_folder))
        .unwrap();
    wait(&|| {
        picker
            .0
            .dialog
            .current_folder()
            .and_then(|folder| folder.path())
            .as_ref()
            == Some(&other_directory.0)
            && visuals.current.borrow().as_ref().is_some_and(|key| {
                key.0.as_deref() == Some(other_folder.child("first.png").uri().as_str())
            })
    });
    picker.close();
    assert!(visuals.task.borrow().is_none());
    assert!(application.windows().is_empty());
    let request = crate::portal::request::Request::parse(
        "SaveFile",
        "Save an image".into(),
        String::new(),
        crate::portal::request::Options::from([
            (
                "current_folder".into(),
                [directory.0.as_os_str().as_encoded_bytes(), &[0]]
                    .concat()
                    .to_variant(),
            ),
            ("current_name".into(), "new photo.png".to_variant()),
            (
                "filters".into(),
                vec![("*.png".to_string(), vec![(0_u32, "*.png".to_string())])].to_variant(),
            ),
        ]),
    )
    .unwrap();
    let save = super::super::Picker::new(&application, request, Rc::new(|_| {}));
    wait(&|| save.0.dialog.width() > 0 && !find_file_images(save.0.dialog.upcast_ref()).is_empty());
    assert!(!save.0.visuals.widget.is_visible());
    assert_eq!(
        save.0.dialog.current_name().as_deref(),
        Some("new photo.png")
    );
    if let Some(path) = std::env::var_os("HERMES_PICKER_SCREENSHOT") {
        snapshot_picker(
            &save.0.dialog,
            &theme.applied_tokens().unwrap().background,
            &std::path::PathBuf::from(&path).with_file_name("save-picker.png"),
        );
        theme.select_theme("paper");
        wait(&|| {
            save.0.visuals.kind.color()
                == gtk::gdk::RGBA::parse(theme.applied_tokens().unwrap().text).unwrap()
        });
        snapshot_picker(
            &save.0.dialog,
            &theme.applied_tokens().unwrap().background,
            &std::path::PathBuf::from(&path).with_file_name("save-picker-light.png"),
        );
    }
    save.close();
    assert!(application.windows().is_empty());
}

fn snapshot_picker(dialog: &gtk::FileChooserDialog, background: &str, path: &std::path::Path) {
    let deadline = std::time::Instant::now() + Duration::from_millis(150);
    while std::time::Instant::now() < deadline {
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(3));
    }
    dialog.allocate(1080, 720, -1, None);
    invalidate_snapshots(dialog.upcast_ref());
    let snapshot = gtk::Snapshot::new();
    snapshot.append_color(
        &gtk::gdk::RGBA::parse(background).unwrap(),
        &gtk::graphene::Rect::new(0.0, 0.0, 1080.0, 720.0),
    );
    let mut child = dialog.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        dialog.snapshot_child(&widget, &snapshot);
    }
    let node = snapshot.to_node().unwrap();
    let renderer = gtk::gsk::CairoRenderer::new();
    renderer.realize(None::<&gtk::gdk::Surface>).unwrap();
    renderer
        .render_texture(&node, None)
        .save_to_png(path)
        .unwrap();
    renderer.unrealize();
}

fn invalidate_snapshots(widget: &gtk::Widget) {
    let _color = widget.color();
    widget.queue_draw();
    let mut child = widget.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        invalidate_snapshots(&widget);
    }
}

fn describe_widgets(widget: &gtk::Widget, depth: usize, output: &mut String) {
    use std::fmt::Write;
    if widget.type_().name() == "GtkFileThumbnail" {
        writeln!(
            output,
            "THUMB properties {:?}",
            widget
                .list_properties()
                .iter()
                .map(|property| property.name())
                .collect::<Vec<_>>()
        )
        .unwrap();
    }
    writeln!(
        output,
        "{}{} {} {:?}",
        "  ".repeat(depth),
        widget.type_().name(),
        widget.css_name(),
        widget.css_classes()
    )
    .unwrap();
    if let Some(view) = widget.downcast_ref::<gtk::ColumnView>() {
        for position in 0..view.columns().n_items() {
            let column = view
                .columns()
                .item(position)
                .and_downcast::<gtk::ColumnViewColumn>()
                .unwrap();
            writeln!(
                output,
                "COLUMN {position} factory {:?}",
                column.factory().map(|factory| factory.type_().name())
            )
            .unwrap();
        }
    }
    if let Some(view) = widget.downcast_ref::<gtk::GridView>() {
        writeln!(
            output,
            "GRID factory {:?}",
            view.factory().map(|factory| factory.type_().name())
        )
        .unwrap();
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        describe_widgets(&widget, depth + 1, output);
    }
}

fn find_file_images(widget: &gtk::Widget) -> Vec<gtk::Image> {
    let mut images = Vec::new();
    if widget.has_css_class("picker-file-icon")
        && let Some(image) = widget.downcast_ref::<gtk::Image>()
    {
        images.push(image.clone());
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        images.extend(find_file_images(&widget));
    }
    images
}
