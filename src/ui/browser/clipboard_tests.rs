use super::*;

#[test]
fn native_clipboard_encodes_directory_uris_without_inspecting_files() {
    let locations = [
        Location::local("/generated/Folder ü #100%"),
        Location::uri("sftp://host.example/home/user/Second%20folder"),
    ];
    assert_eq!(file_clipboard_uri_list(&locations).unwrap().as_ref(),
        b"file:///generated/Folder%20%C3%BC%20%23100%25\r\nsftp://host.example/home/user/Second%20folder\r\n");
    assert!(file_clipboard_uri_list(&[]).is_none());
}

#[test]
fn native_clipboard_preserves_non_utf8_directory_names() {
    use std::os::unix::ffi::OsStringExt;
    let location = Location::local(PathBuf::from(std::ffi::OsString::from_vec(
        b"/generated/folder-\xff".to_vec(),
    )));
    assert_eq!(
        file_clipboard_uri_list(&[location]).unwrap().as_ref(),
        b"file:///generated/folder-%FF\r\n"
    );
}

#[track_caller]
fn pump_until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        for _ in 0..128 {
            if !glib::MainContext::default().iteration(false) {
                break;
            }
        }
        if condition() {
            return;
        }
        assert!(Instant::now() < deadline, "Clipboard fixture timed out");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn run_client(command: &mut Command) -> std::process::ExitStatus {
    let mut child = ClipboardClient(command.spawn().unwrap());
    let mut status = None;
    pump_until(|| {
        status = child.0.try_wait().unwrap();
        status.is_some()
    });
    status.unwrap()
}

fn client_text(command: &mut Command) -> (std::process::ExitStatus, String) {
    use std::io::Read;
    let mut child = ClipboardClient(
        command
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    );
    let mut status = None;
    pump_until(|| {
        status = child.0.try_wait().unwrap();
        status.is_some()
    });
    let mut output = String::new();
    child
        .0
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut output)
        .unwrap();
    (status.unwrap(), output)
}

struct ClipboardClient(std::process::Child);

impl Drop for ClipboardClient {
    fn drop(&mut self) {
        let _result = self.0.kill();
        let _result = self.0.wait();
    }
}

fn menu_button(widget: &gtk::Widget, title: &str) -> Option<gtk::Button> {
    if let Some(button) = widget.downcast_ref::<gtk::Button>()
        && button
            .child()
            .and_then(|row| row.first_child())
            .and_then(|icon| icon.next_sibling())
            .and_downcast::<gtk::Label>()
            .is_some_and(|label| label.text() == title)
    {
        return Some(button.clone());
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        child = widget.next_sibling();
        if widget.is_visible()
            && let Some(button) = menu_button(&widget, title)
        {
            return Some(button);
        }
    }
    None
}

fn build_kde_clipboard_probe(root: &Path) -> PathBuf {
    let source = root.join("kde-probe");
    std::fs::create_dir(&source).unwrap();
    std::fs::write(
        source.join("CMakeLists.txt"),
        r#"
cmake_minimum_required(VERSION 3.20)
project(hermes_clipboard_probe LANGUAGES CXX)
find_package(Qt6 REQUIRED COMPONENTS Widgets)
find_package(KF6KIO REQUIRED)
find_package(KF6CoreAddons REQUIRED)
add_executable(probe probe.cpp)
target_link_libraries(probe PRIVATE Qt6::Widgets KF6::KIOWidgets KF6::CoreAddons)
"#,
    )
    .unwrap();
    std::fs::write(source.join("probe.cpp"), r#"
#include <QApplication>
#include <QClipboard>
#include <QMimeData>
#include <QTimer>
#include <QWidget>
#include <KUrlMimeData>
#include <KFileItem>
#include <KIO/Paste>
#include <KIO/PasteJob>
#include <cstdio>

int main(int argc, char **argv)
{
    QApplication app(argc, argv);
    QWidget window;
    window.setWindowTitle("Private KDE clipboard probe");
    window.show();
    QTimer::singleShot(250, &app, [&] {
        const auto *mime = app.clipboard()->mimeData();
        const auto urls = KUrlMimeData::urlsFromMimeData(mime);
        bool enabled = false;
        const auto action = KIO::pasteActionText(mime, &enabled, KFileItem(QUrl::fromLocalFile(argv[1])));
        fprintf(stderr, "%s: %d URLs; formats: %s\n", action.toUtf8().constData(), int(urls.size()), mime->formats().join(", ").toUtf8().constData());
        if (!enabled || urls.size() != QString::fromUtf8(argv[2]).toInt()
            || mime->data("text/uri-list") != mime->data("application/x-kde4-urilist")
            || mime->hasFormat("application/vnd.portal.filetransfer")) {
            app.exit(2);
            return;
        }
        auto *job = KIO::paste(mime, QUrl::fromLocalFile(argv[1]), KIO::HideProgressInfo);
        QObject::connect(job, &KJob::result, &app, [&](KJob *result) {
            fprintf(stderr, "KIO paste result: %d\n", result->error());
            app.exit(result->error() ? 3 : 0);
        });
    });
    QTimer::singleShot(15000, &app, [&] { app.exit(4); });
    return app.exec();
}
"#).unwrap();
    let build = source.join("build");
    for command in [
        Command::new("cmake")
            .arg("-S")
            .arg(&source)
            .arg("-B")
            .arg(&build),
        Command::new("cmake").arg("--build").arg(&build),
    ] {
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    build.join("probe")
}

#[test]
#[ignore = "Requires a private desktop/session bus, private XDG dirs, Dolphin, qdbus6, CMake and Qt6/KF6 development files; pastes generated folders using installed KIO and Dolphin"]
fn desktop_directory_copy_pastes_into_dolphin() {
    use crate::services::{DirectoryEvent, DirectoryRequest, LoadHandle};

    let config = PathBuf::from(std::env::var_os("XDG_CONFIG_HOME").unwrap());
    let runtime = PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR").unwrap());
    assert!(config.starts_with(std::env::temp_dir()));
    assert!(runtime.starts_with(std::env::temp_dir()));
    for variable in ["HOME", "XDG_DATA_HOME", "XDG_CACHE_HOME", "XDG_STATE_HOME"] {
        assert!(
            PathBuf::from(std::env::var_os(variable).unwrap())
                .starts_with(config.parent().unwrap())
        );
    }
    assert!(
        std::env::var("DBUS_SESSION_BUS_ADDRESS")
            .unwrap()
            .starts_with("unix:path=/tmp/")
    );
    assert!(!crate::services::actions_path().exists());
    let root = config
        .parent()
        .unwrap()
        .join(format!("directory-copy-{}", std::process::id()));
    assert!(!root.exists());
    std::fs::create_dir_all(&root).unwrap();
    let probe = build_kde_clipboard_probe(&root);
    let source = root.join("source");
    let names = ["Folder ü #100%", "Second folder"];
    let entries = names
        .iter()
        .map(|name| {
            let directory = source.join(name);
            std::fs::create_dir_all(directory.join("nested")).unwrap();
            std::fs::create_dir_all(directory.join("empty")).unwrap();
            std::fs::write(directory.join("nested/payload.txt"), name.as_bytes()).unwrap();
            FileEntry(Rc::new(crate::model::FileEntryInner {
                location: Location::local(directory),
                native_name: (*name).into(),
                display_name: (*name).into(),
                kind: EntryKind::Directory,
                size: crate::model::MetadataValue::Unknown,
                modified_unix_seconds: crate::model::MetadataValue::Unknown,
                music_tags: None,
            }))
        })
        .collect::<Vec<_>>();
    struct GeneratedSource(Vec<FileEntry>);
    impl FileSource for GeneratedSource {
        fn validate_location(
            &self,
            _: &Location,
        ) -> Result<(), crate::services::LocationValidationError> {
            Ok(())
        }
        fn enumerate(
            &self,
            request: DirectoryRequest,
            emit: Rc<dyn Fn(DirectoryEvent)>,
        ) -> LoadHandle {
            emit(DirectoryEvent::Batch {
                request_id: request.id,
                entries: self.0.clone(),
            });
            emit(DirectoryEvent::Finished {
                request_id: request.id,
            });
            LoadHandle::new(|| {})
        }
    }
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let view = BrowserView::new(Rc::new(GeneratedSource(entries)), PeekBehavior::default());
    view.browser().navigate(Location::local(&source));
    pump_until(|| view.browser().entry_at(0, 1).is_some());
    let parent = gtk::Box::new(gtk::Orientation::Vertical, 0);
    parent.append(&gtk::Label::new(Some("Generated folder target")));
    let selection = gtk::MultiSelection::new(Some(gtk::StringList::new(&names)));
    install_item_context_menu(
        &view.state,
        parent.upcast_ref(),
        &selection,
        Rc::new(|_| Some(0)),
        Rc::new(|position| Some(position as usize)),
        0,
    );
    let window = gtk::Window::builder()
        .title("Hermes clipboard fixture")
        .default_width(500)
        .default_height(300)
        .child(&parent)
        .build();
    window.present();
    pump_until(|| parent.width() > 0);
    let controllers = parent.observe_controllers();
    let click = (0..controllers.n_items())
        .filter_map(|position| {
            controllers
                .item(position)
                .and_downcast::<gtk::GestureClick>()
        })
        .next()
        .unwrap();
    for count in [1, 2] {
        if let Some(input) = std::env::var_os("HERMES_CLIPBOARD_QA_INPUT") {
            let input = PathBuf::from(input);
            assert!(input.starts_with(std::env::temp_dir()));
            window.present();
            pump_until(|| window.is_active());
            assert!(run_client(&mut Command::new(input)).success());
        }
        selection.select_item(0, true);
        if count == 2 {
            selection.select_all();
        }
        click.emit_by_name::<()>("pressed", &[&1i32, &12.0f64, &12.0f64]);
        menu_button(parent.upcast_ref(), "Copy")
            .expect("Ordinary folder Copy, not Copy to")
            .emit_clicked();
        assert!(view.state.cut_locations.borrow().is_empty());
        let clipboard = gtk::gdk::Display::default().unwrap().clipboard();
        let formats = clipboard.formats();
        assert!(formats.contain_mime_type("text/uri-list"));
        assert!(formats.contain_mime_type("application/x-kde4-urilist"));
        assert!(!formats.contain_mime_type("application/vnd.portal.filetransfer"));
        assert!(!formats.contain_mime_type("application/vnd.portal.files"));
        assert!(clipboard_has_files());
        let content = clipboard.content();
        assert!(!set_location_files_clipboard(&[]));
        assert_eq!(clipboard.content(), content);
        let context = glib::MainContext::default();
        let files = context
            .block_on(
                clipboard
                    .read_value_future(gtk::gdk::FileList::static_type(), glib::Priority::DEFAULT),
            )
            .unwrap()
            .get::<gtk::gdk::FileList>()
            .unwrap()
            .files();
        assert_eq!(files.len(), count);
        for file in &files {
            assert!(file.path().unwrap().starts_with(&source));
        }
        for client in ["kio", "dolphin"] {
            let destination = root.join(format!("{client}-{count}"));
            std::fs::create_dir(&destination).unwrap();
            if client == "kio" {
                assert!(
                    run_client(
                        Command::new(&probe)
                            .arg(&destination)
                            .arg(count.to_string())
                    )
                    .success()
                );
            } else {
                eprintln!(
                    "Opening isolated Dolphin destination: {}",
                    destination.display()
                );
                let dolphin = ClipboardClient(
                    Command::new("dolphin")
                        .env(
                            "QT_QPA_PLATFORM",
                            std::env::var("HERMES_DOLPHIN_QPA_PLATFORM")
                                .unwrap_or_else(|_| "xcb".into()),
                        )
                        .arg("--new-window")
                        .arg(&destination)
                        .spawn()
                        .unwrap(),
                );
                let service = format!("org.kde.dolphin-{}", dolphin.0.id());
                let action = "/dolphin/Dolphin_1/actions/edit_paste";
                pump_until(|| {
                    let (status, enabled) = client_text(Command::new("qdbus6").args([
                        &service,
                        action,
                        "org.freedesktop.DBus.Properties.Get",
                        "org.qtproject.Qt.QAction",
                        "enabled",
                    ]));
                    status.success() && enabled.trim() == "true"
                });
                let (status, title) = client_text(Command::new("qdbus6").args([
                    &service,
                    action,
                    "org.freedesktop.DBus.Properties.Get",
                    "org.qtproject.Qt.QAction",
                    "text",
                ]));
                assert!(status.success());
                eprintln!("Triggering actual Dolphin Paste action: {}", title.trim());
                assert!(
                    run_client(Command::new("qdbus6").args([
                        &service,
                        action,
                        "org.qtproject.Qt.QAction.trigger",
                    ]))
                    .success()
                );
                pump_until(|| {
                    names
                        .iter()
                        .take(count)
                        .all(|name| destination.join(name).join("nested/payload.txt").is_file())
                });
            }
            for name in names.iter().take(count) {
                assert_eq!(
                    std::fs::read(destination.join(name).join("nested/payload.txt")).unwrap(),
                    name.as_bytes()
                );
                assert!(destination.join(name).join("empty").is_dir());
                assert_eq!(
                    std::fs::read(source.join(name).join("nested/payload.txt")).unwrap(),
                    name.as_bytes()
                );
            }
            assert_eq!(std::fs::read_dir(&destination).unwrap().count(), count);
            eprintln!("Verified {client} ordinary Copy/paste: {count} generated folder(s)");
        }
    }
    window.close();
    std::fs::remove_dir_all(root).unwrap();
}
