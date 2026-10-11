use super::*;

#[test]
#[ignore = "Requires GTK desktop display; uses generated widgets, not personal documents"]
fn desktop_pdf_fullscreen_reuses_reader_and_restores_on_exit_and_close() {
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    content.set_size_request(600, 4000);
    let scroll = gtk::ScrolledWindow::builder()
        .child(&content)
        .vexpand(true)
        .build();
    let reader = PdfFullscreen::new(&scroll, "Fixture document.pdf");
    let window = gtk::Window::builder()
        .default_width(700)
        .default_height(500)
        .child(&reader.widget)
        .build();
    window.present();
    let pump = |predicate: &dyn Fn() -> bool| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !predicate() && std::time::Instant::now() < deadline {
            glib::MainContext::default().iteration(false);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(predicate());
    };
    pump(&|| scroll.height() > 0);
    scroll.vadjustment().set_value(1300.0);
    let viewer = reader.widget.first_child().unwrap();
    assert!(reader.open_fullscreen());
    let fullscreen = reader.window.borrow().clone().unwrap();
    assert_eq!(fullscreen.child().as_ref(), Some(&viewer));
    assert!(reader.widget.first_child().is_none());
    pump(&|| fullscreen.is_fullscreen() && scroll.vadjustment().value() > 1200.0);
    let keys = fullscreen
        .observe_controllers()
        .iter::<glib::Object>()
        .filter_map(Result::ok)
        .filter_map(|object| object.downcast::<gtk::EventControllerKey>().ok())
        .find(|keys| keys.name().as_deref() == Some("hermes-pdf-fullscreen"))
        .unwrap();
    assert!(!keys.emit_by_name::<bool>(
        "key-pressed",
        &[
            &gtk::gdk::Key::f,
            &0_u32,
            &gtk::gdk::ModifierType::CONTROL_MASK
        ]
    ));
    assert!(keys.emit_by_name::<bool>(
        "key-pressed",
        &[&gtk::gdk::Key::f, &0_u32, &gtk::gdk::ModifierType::empty()]
    ));
    assert!(reader.window.borrow().is_none());
    assert_eq!(reader.widget.first_child().as_ref(), Some(&viewer));
    let viewport = scroll.child().and_downcast::<gtk::Viewport>().unwrap();
    assert_eq!(viewport.child().as_ref(), Some(content.upcast_ref()));
    assert!(reader.open_fullscreen());
    reader.stop();
    assert!(reader.window.borrow().is_none());
    assert_eq!(reader.widget.first_child().as_ref(), Some(&viewer));
    window.close();

    struct Pages(Vec<u8>);
    impl crate::services::PreviewProvider for Pages {
        fn load(
            &self,
            request: crate::services::PreviewRequest,
            emit: Rc<dyn Fn(crate::services::PreviewEvent)>,
        ) -> crate::services::LoadHandle {
            let active = Rc::new(Cell::new(true));
            let pending = active.clone();
            let png = self.0.clone();
            glib::idle_add_local_once(move || {
                if pending.get() {
                    emit(crate::services::PreviewEvent::Ready(
                        crate::services::Preview {
                            request_id: request.id,
                            entry: request.entry,
                            content_type: "application/pdf".into(),
                            content: crate::services::PreviewContent::Pdf {
                                png,
                                page: request.pdf_page,
                                pages: 8,
                                text_layer: None,
                            },
                        },
                    ));
                }
            });
            crate::services::LoadHandle::new(move || active.set(false))
        }
    }
    let mut png = Vec::new();
    cairo::ImageSurface::create(cairo::Format::ARgb32, 500, 700)
        .unwrap()
        .write_to_png(&mut png)
        .unwrap();
    let drawer = super::super::PreviewDrawer::new(Rc::new(Pages(png)));
    let entry = crate::model::FileEntry(Rc::new(crate::model::FileEntryInner {
        location: crate::model::Location::local("/fixture/document.pdf"),
        native_name: "document.pdf".into(),
        display_name: "document.pdf".into(),
        kind: crate::model::EntryKind::File,
        size: crate::model::MetadataValue::Unknown,
        modified_unix_seconds: crate::model::MetadataValue::Unknown,
        music_tags: None,
    }));
    let host = gtk::Window::builder()
        .default_width(750)
        .default_height(600)
        .child(&drawer.widget())
        .build();
    host.present();
    drawer.show(entry);
    pump(&|| drawer.state.pdf.borrow().is_some());
    assert!(drawer.open_fullscreen());
    let fullscreen = drawer
        .state
        .pdf
        .borrow()
        .as_ref()
        .unwrap()
        .window
        .borrow()
        .clone()
        .unwrap();
    pump(&|| fullscreen.is_visible());
    drawer.close();
    assert!(!fullscreen.is_visible());
    assert!(drawer.state.pdf.borrow().is_none());
    assert!(drawer.state.content.first_child().is_none());
    host.close();
}
