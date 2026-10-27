use super::*;

struct Pending(RefCell<Vec<(bool, PreviewRequest, Rc<dyn Fn(PreviewEvent)>)>>);

impl Pending {
    fn record(
        &self,
        layout: bool,
        request: PreviewRequest,
        emit: Rc<dyn Fn(PreviewEvent)>,
    ) -> LoadHandle {
        self.0.borrow_mut().push((layout, request, emit));
        LoadHandle::new(|| {})
    }
}

impl PreviewProvider for Pending {
    fn load(&self, request: PreviewRequest, emit: Rc<dyn Fn(PreviewEvent)>) -> LoadHandle {
        self.record(false, request, emit)
    }

    fn load_layout(&self, request: PreviewRequest, emit: Rc<dyn Fn(PreviewEvent)>) -> LoadHandle {
        self.record(true, request, emit)
    }
}

fn fixture(name: &str) -> FileEntry {
    FileEntry(Rc::new(crate::model::FileEntryInner {
        location: crate::model::Location::local(format!("/generated/{name}")),
        native_name: name.into(),
        display_name: name.into(),
        music_tags: None,
        kind: crate::model::EntryKind::File,
        size: MetadataValue::Known(1024),
        modified_unix_seconds: MetadataValue::Unknown,
    }))
}

fn button(root: &gtk::Widget, label: &str) -> Option<gtk::ToggleButton> {
    if let Some(button) = root.downcast_ref::<gtk::ToggleButton>() {
        if button.label().as_deref() == Some(label) {
            return Some(button.clone());
        }
    }
    let mut child = root.first_child();
    while let Some(widget) = child {
        if let Some(found) = button(&widget, label) {
            return Some(found);
        }
        child = widget.next_sibling();
    }
    None
}

#[test]
fn workbook_layout_support_excludes_legacy_and_non_workbook_formats() {
    for name in ["budget.XLSX", "budget.ods"] {
        assert!(supports_layout(&fixture(name)));
    }
    for name in ["legacy.xls", "slides.pptx", "document.docx"] {
        assert!(!supports_layout(&fixture(name)));
    }
}

#[test]
#[ignore = "Private GTK display and installed renderer; synthetic workbook UI, no real conversion"]
fn desktop_workbook_modes_cancel_stale_layouts_and_fall_back_to_data() {
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    assert!(crate::sandbox::office::available());
    let provider = Rc::new(Pending(RefCell::new(Vec::new())));
    let drawer = PreviewDrawer::new(provider.clone());
    let window = gtk::Window::builder()
        .child(&drawer.widget())
        .default_width(420)
        .default_height(500)
        .build();
    window.present();
    let entry = fixture("budget.xlsx");
    drawer.show(entry.clone());
    let (layout, first_request, first_emit) = provider.0.borrow()[0].clone();
    assert!(layout);
    button(drawer.state.content.upcast_ref(), "Data")
        .unwrap()
        .emit_clicked();
    let (layout, data_request, data_emit) = provider.0.borrow().last().unwrap().clone();
    assert!(!layout);
    assert_ne!(data_request.id, first_request.id);
    let data = PreviewContent::Spreadsheet {
        table: crate::services::table::SpreadsheetData::default(),
    };
    data_emit(PreviewEvent::Ready(Preview {
        request_id: data_request.id,
        entry: entry.clone(),
        content_type: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet".into(),
        content: data.clone(),
    }));
    assert!(drawer.state.table.borrow().is_some());
    let pixels = gdk_pixbuf::Pixbuf::new(gdk_pixbuf::Colorspace::Rgb, false, 8, 640, 420).unwrap();
    pixels.fill(0x204080ff);
    let pdf = PreviewContent::Pdf {
        png: pixels.save_to_bufferv("png", &[]).unwrap(),
        page: 0,
        pages: 2,
        text_layer: None,
    };
    first_emit(PreviewEvent::Ready(Preview {
        request_id: first_request.id,
        entry: entry.clone(),
        content_type: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet".into(),
        content: pdf.clone(),
    }));
    assert!(drawer.state.table.borrow().is_some());
    assert!(drawer.state.pdf.borrow().is_none());
    button(drawer.state.content.upcast_ref(), "Layout")
        .unwrap()
        .emit_clicked();
    let (layout, request, emit) = provider.0.borrow().last().unwrap().clone();
    assert!(layout);
    emit(PreviewEvent::Failed {
        request_id: request.id,
        entry: entry.clone(),
        message: "Generated converter failure".into(),
    });
    let (layout, request, emit) = provider.0.borrow().last().unwrap().clone();
    assert!(!layout);
    emit(PreviewEvent::Ready(Preview {
        request_id: request.id,
        entry: entry.clone(),
        content_type: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet".into(),
        content: data,
    }));
    assert!(drawer.state.layout_failure.borrow().is_some());
    assert!(drawer.state.table.borrow().is_some());
    button(drawer.state.content.upcast_ref(), "Layout")
        .unwrap()
        .emit_clicked();
    let (layout, request, emit) = provider.0.borrow().last().unwrap().clone();
    assert!(layout);
    emit(PreviewEvent::Ready(Preview {
        request_id: request.id,
        entry,
        content_type: "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet".into(),
        content: pdf,
    }));
    assert!(drawer.state.pdf.borrow().is_some());
    assert!(drawer.state.table.borrow().is_none());
    assert!(drawer.state.layout_failure.borrow().is_none());
    fn find_layout_scroll(root: &gtk::Widget) -> Option<gtk::ScrolledWindow> {
        if root.has_css_class("spreadsheet-layout-scroll") {
            return root.clone().downcast().ok();
        }
        let mut child = root.first_child();
        while let Some(widget) = child {
            if let Some(scroll) = find_layout_scroll(&widget) {
                return Some(scroll);
            }
            child = widget.next_sibling();
        }
        None
    }
    let scroll = find_layout_scroll(drawer.state.content.upcast_ref()).unwrap();
    assert_eq!(scroll.hscrollbar_policy(), gtk::PolicyType::Always);
    assert_eq!(scroll.vscrollbar_policy(), gtk::PolicyType::Always);
    assert!(!scroll.is_overlay_scrolling());
    let deadline = Instant::now() + Duration::from_millis(150);
    while Instant::now() < deadline {
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
    for bar in [scroll.hscrollbar(), scroll.vscrollbar()] {
        assert!(bar.is_mapped());
        let bounds = bar.compute_bounds(&scroll).unwrap();
        assert!(pdf_scrollbar_at(
            &scroll,
            f64::from(bounds.x() + bounds.width() / 2.0),
            f64::from(bounds.y() + bounds.height() / 2.0)
        ));
    }
    drawer.close();
    window.close();
    assert!(drawer.state.pdf.borrow().is_none());
    assert!(drawer.state.loading_task.borrow().is_none());
}
