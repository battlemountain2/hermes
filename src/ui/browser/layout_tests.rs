use super::*;
use crate::services::{
    DirectoryEvent, DirectoryRequest, LoadHandle, PreviewEvent, PreviewProvider, PreviewRequest,
};

struct GeneratedSource;

impl FileSource for GeneratedSource {
    fn validate_location(
        &self,
        _: &Location,
    ) -> Result<(), crate::services::LocationValidationError> {
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        emit(DirectoryEvent::Finished {
            request_id: request.id,
        });
        LoadHandle::new(|| {})
    }
}

struct PendingPreview;

impl PreviewProvider for PendingPreview {
    fn load(&self, _: PreviewRequest, _: Rc<dyn Fn(PreviewEvent)>) -> LoadHandle {
        LoadHandle::new(|| {})
    }
}

fn pump(window: &gtk::Window) {
    let until = Instant::now() + Duration::from_millis(450);
    while Instant::now() < until {
        while glib::MainContext::default().iteration(false) {}
        window.queue_draw();
        if let Some(clock) = window.frame_clock() {
            clock.emit_by_name::<()>("update", &[]);
            clock.emit_by_name::<()>("layout", &[]);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn assert_no_trailing_gap(view: &BrowserView) {
    let root = view.state.columns_widget.borrow();
    let scroller = view.state.scroller.borrow();
    let horizontal = scroller.hadjustment();
    let columns = view.state.columns.borrow();
    let last = columns.last().unwrap();
    let bounds = last.shell.compute_bounds(&*root).unwrap();
    assert_eq!(
        bounds.x().round() as i32 + bounds.width().round() as i32,
        root.width()
    );
    assert!(f64::from(root.width()) >= horizontal.page_size() - 1.0);
    assert!(last.shell.hexpands());
    for column in &columns[..columns.len() - 1] {
        assert!(!column.shell.hexpands());
        assert_eq!(column.shell.width(), column.shell.width_request());
    }
}

#[test]
#[ignore = "Private GTK display; generated column/divider/retained-tab geometry"]
fn desktop_last_column_fills_resized_preview_boundary_without_feedback() {
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    let _theme = crate::ui::theme::ThemeManager::shared();
    let styles = gtk::CssProvider::new();
    styles.load_from_string(include_str!("../../style.css"));
    let display = gtk::gdk::Display::default().unwrap();
    gtk::style_context_add_provider_for_display(
        &display,
        &styles,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    let view = BrowserView::new(Rc::new(GeneratedSource), PeekBehavior::default());
    let drawer = crate::ui::preview::PreviewDrawer::new(Rc::new(PendingPreview));
    let split = gtk::Paned::new(gtk::Orientation::Horizontal);
    split.set_shrink_start_child(true);
    split.set_shrink_end_child(true);
    split.set_start_child(Some(&view.widget()));
    split.set_end_child(Some(&drawer.widget()));
    let measured = view.clone();
    drawer.attach_split(
        &split,
        Rc::new(move || measured.preview_occupied_width(1200)),
    );
    let window = gtk::Window::builder()
        .default_width(1200)
        .default_height(650)
        .child(&split)
        .build();
    window.present();
    view.navigate("/generated-layout/First");
    pump(&window);
    assert_no_trailing_gap(&view);
    assert_eq!(view.preview_occupied_width(1200), COLUMN_WIDTH);
    drawer.show(FileEntry(Rc::new(crate::model::FileEntryInner {
        location: Location::local("/generated-layout/preview.png"),
        native_name: "preview.png".into(),
        display_name: "preview.png".into(),
        kind: EntryKind::File,
        size: crate::model::MetadataValue::Unknown,
        modified_unix_seconds: crate::model::MetadataValue::Unknown,
        music_tags: None,
    })));
    pump(&window);
    for count in 1..=3 {
        if count > 1 {
            view.state.append_column(
                count - 1,
                &Location::local(format!("/generated-layout/Folder{count}")),
            );
            pump(&window);
        }
        assert_eq!(
            view.preview_occupied_width(1200),
            count as i32 * COLUMN_WIDTH
        );
        for position in [800, 600, 350] {
            split.set_position(position);
            pump(&window);
            let stable_position = split.position();
            pump(&window);
            assert_eq!(
                split.position(),
                stable_position,
                "Expanded allocation cannot move the preview divider back"
            );
            assert_no_trailing_gap(&view);
            assert_eq!(
                view.preview_occupied_width(1200),
                count as i32 * COLUMN_WIDTH
            );
        }
    }
    let horizontal = view.state.scroller.borrow().hadjustment();
    assert!(horizontal.upper() > horizontal.page_size());
    horizontal.set_value(horizontal.upper() - horizontal.page_size());
    pump(&window);
    assert_no_trailing_gap(&view);
    view.state.truncate(1);
    pump(&window);
    assert_no_trailing_gap(&view);
    view.state.columns.borrow()[0].shell.set_width_request(410);
    pump(&window);
    assert_eq!(view.preview_occupied_width(1200), 410);
    assert_no_trailing_gap(&view);
    let retained_id = TrailId::new("layout-first").unwrap();
    view.state.store_presentation(retained_id.clone());
    view.state
        .append_column(0, &Location::local("/generated-layout/Second"));
    pump(&window);
    assert_no_trailing_gap(&view);
    assert_eq!(view.preview_occupied_width(1200), COLUMN_WIDTH);
    view.state.restore_presentation(&retained_id);
    pump(&window);
    assert_eq!(view.preview_occupied_width(1200), 410);
    assert_no_trailing_gap(&view);
    drawer.close();
    pump(&window);
    assert_no_trailing_gap(&view);
    window.close();
    gtk::style_context_remove_provider_for_display(&display, &styles);
}
