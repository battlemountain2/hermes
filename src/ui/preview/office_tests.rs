use super::*;

#[test]
#[ignore = "Private GTK display; generated Office page data, no converter or personal documents"]
fn desktop_office_layout_reuses_document_reader_and_fallback_explains_missing_renderer() {
    struct Pending(Rc<RefCell<Vec<PreviewRequest>>>);
    impl PreviewProvider for Pending {
        fn load(&self, request: PreviewRequest, _: Rc<dyn Fn(PreviewEvent)>) -> LoadHandle {
            self.0.borrow_mut().push(request);
            LoadHandle::new(|| {})
        }
    }
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let requests = Rc::new(RefCell::new(Vec::new()));
    let drawer = PreviewDrawer::new(Rc::new(Pending(requests.clone())));
    let window = gtk::Window::builder()
        .default_width(700)
        .default_height(700)
        .child(&drawer.widget())
        .build();
    window.present();
    let entry = FileEntry(Rc::new(crate::model::FileEntryInner {
        music_tags: None,
        location: crate::model::Location::local("/generated/Presentation.PPTX"),
        native_name: "Presentation.PPTX".into(),
        display_name: "Presentation.PPTX".into(),
        kind: crate::model::EntryKind::File,
        size: MetadataValue::Known(1024),
        modified_unix_seconds: MetadataValue::Unknown,
    }));
    drawer.show(entry.clone());
    let pixels = gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, 640, 360).unwrap();
    pixels.fill(0x204080ff);
    drawer.state.render(Preview {
        request_id: PreviewRequestId(1),
        entry: entry.clone(),
        content_type: "application/vnd.openxmlformats-officedocument.presentationml.presentation"
            .into(),
        content: PreviewContent::Pdf {
            png: pixels.save_to_bufferv("png", &[]).unwrap(),
            page: 0,
            pages: 2,
            text_layer: None,
        },
    });
    let label = drawer
        .state
        .content
        .first_child()
        .unwrap()
        .downcast::<gtk::Label>()
        .unwrap();
    assert_eq!(label.text(), "LibreOffice layout · 2 slides");
    assert!(drawer.state.pdf.borrow().is_some());
    assert_eq!(drawer.state.title.text(), "Presentation.PPTX");
    let until = Instant::now() + Duration::from_millis(350);
    while Instant::now() < until {
        for _ in 0..64 {
            if !glib::MainContext::default().iteration(false) {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(
        requests
            .borrow()
            .iter()
            .all(|request| request.entry.location == entry.location)
    );
    drawer.state.render(Preview {
        request_id: PreviewRequestId(2),
        entry,
        content_type: "application/vnd.openxmlformats-officedocument.presentationml.presentation"
            .into(),
        content: PreviewContent::OfficeText {
            content: "Readable slide text".into(),
            truncated: false,
            reason:
                "Install LibreOffice for page and slide layouts. Showing extracted text instead."
                    .into(),
        },
    });
    assert!(drawer.state.pdf.borrow().is_none());
    let label = drawer
        .state
        .content
        .first_child()
        .unwrap()
        .downcast::<gtk::Label>()
        .unwrap();
    assert!(label.text().contains("Install LibreOffice"));
    let scroll = label
        .next_sibling()
        .unwrap()
        .downcast::<gtk::ScrolledWindow>()
        .unwrap();
    let text = scroll
        .child()
        .unwrap()
        .downcast::<sourceview5::View>()
        .unwrap()
        .buffer();
    assert_eq!(
        text.text(&text.start_iter(), &text.end_iter(), false),
        "Readable slide text"
    );
    drawer.close();
    window.close();
}
