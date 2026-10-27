use super::*;
use crate::services::{DirectoryEvent, DirectoryRequest, LoadHandle};

struct Request {
    directory: DirectoryRequest,
    emit: Rc<dyn Fn(DirectoryEvent)>,
    cancelled: Rc<Cell<bool>>,
}

impl Request {
    fn batch(&self, name: &str) {
        let entry = FileEntry(Rc::new(crate::model::FileEntryInner {
            location: Location::local(self.directory.location.native_path().unwrap().join(name)),
            native_name: name.into(),
            display_name: name.into(),
            kind: EntryKind::File,
            size: crate::model::MetadataValue::Unknown,
            modified_unix_seconds: crate::model::MetadataValue::Unknown,
            music_tags: None,
        }));
        (self.emit)(DirectoryEvent::Batch {
            request_id: self.directory.id,
            entries: vec![entry],
        });
    }

    fn finish(&self) {
        (self.emit)(DirectoryEvent::Finished {
            request_id: self.directory.id,
        });
    }

    fn fail(&self, message: &str) {
        (self.emit)(DirectoryEvent::Failed {
            request_id: self.directory.id,
            message: message.into(),
        });
    }
}

#[derive(Default)]
struct GeneratedSource(RefCell<Vec<Rc<Request>>>);

impl GeneratedSource {
    fn latest(&self) -> Rc<Request> {
        self.0.borrow().last().unwrap().clone()
    }
}

impl FileSource for GeneratedSource {
    fn validate_location(
        &self,
        _: &Location,
    ) -> Result<(), crate::services::LocationValidationError> {
        Ok(())
    }

    fn enumerate(
        &self,
        directory: DirectoryRequest,
        emit: Rc<dyn Fn(DirectoryEvent)>,
    ) -> LoadHandle {
        let cancelled = Rc::new(Cell::new(false));
        self.0.borrow_mut().push(Rc::new(Request {
            directory,
            emit,
            cancelled: cancelled.clone(),
        }));
        LoadHandle::new(move || cancelled.set(true))
    }
}

fn probe(view: &BrowserView) -> (gtk::Stack, gtk::Spinner, gtk::StringList, gtk::Label) {
    if view.view_mode() == BrowserMode::Columns {
        let columns = view.state.columns.borrow();
        let column = columns.last().unwrap();
        (
            column.presentation.stack.clone(),
            column.spinner.clone(),
            column.model.clone(),
            column.presentation.message.clone(),
        )
    } else {
        super::super::browser_modes::tests::load_probe(&view.state.mode_views.borrow())
    }
}

fn pump_loading_deadline() {
    let deadline = Instant::now() + Duration::from_millis(200);
    while Instant::now() < deadline {
        for _ in 0..64 {
            if !glib::MainContext::default().iteration(false) {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn assert_page(stack: &gtk::Stack, name: &str) {
    assert_eq!(stack.visible_child_name().as_deref(), Some(name));
    assert_eq!(stack.transition_type(), gtk::StackTransitionType::None);
}

#[test]
#[ignore = "Requires isolated GTK display; generated folder transitions in all three browser modes"]
fn desktop_folder_transitions_skip_fast_loading_preserve_errors_and_reject_stale_files() {
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    for mode in [
        BrowserMode::Columns,
        BrowserMode::Grid,
        BrowserMode::Explorer,
    ] {
        let source = Rc::new(GeneratedSource::default());
        let view = BrowserView::new(source.clone(), PeekBehavior::default());
        view.set_view_mode(mode);
        let browser = view.browser();
        browser.navigate(Location::local("/generated-transitions/fast"));
        let fast = source.latest();
        let (stack, spinner, _, _) = probe(&view);
        assert_page(&stack, "pending");
        assert!(!spinner.get_visible());
        fast.batch("fast.bin");
        assert_page(&stack, "content");
        pump_loading_deadline();
        assert_page(&stack, "content");
        fast.finish();
        browser.select(0, 0);
        assert_eq!(browser.selected_entries().len(), 1);

        browser.navigate(Location::local("/generated-transitions/slow"));
        assert!(fast.cancelled.get());
        let slow = source.latest();
        let (stack, spinner, model, _) = probe(&view);
        assert_page(&stack, "pending");
        assert_eq!(model.n_items(), 0);
        assert!(browser.selected_entries().is_empty());
        assert!(browser.focused_entry().is_none());
        pump_loading_deadline();
        assert_page(&stack, "loading");
        assert!(spinner.get_visible());
        slow.batch("partial.bin");
        assert_page(&stack, "content");
        slow.fail("Generated provider failure");
        for target_mode in [
            BrowserMode::Grid,
            BrowserMode::Explorer,
            BrowserMode::Columns,
        ] {
            view.set_view_mode(target_mode);
            let (stack, spinner, _, message) = probe(&view);
            assert_page(
                &stack,
                if target_mode == BrowserMode::Columns {
                    "feedback"
                } else {
                    "status"
                },
            );
            assert!(message.text().contains("Generated provider failure"));
            assert!(!spinner.get_visible());
        }
        view.set_view_mode(mode);

        browser.retry_column(0);
        let retry = source.latest();
        let (stack, spinner, model, message) = probe(&view);
        assert_page(&stack, "pending");
        assert_eq!(model.n_items(), 0);
        assert!(!spinner.get_visible());
        slow.batch("late.bin");
        slow.fail("Stale failure");
        assert_page(&stack, "pending");
        assert_eq!(model.n_items(), 0);
        retry.finish();
        assert_page(
            &stack,
            if mode == BrowserMode::Columns {
                "feedback"
            } else {
                "status"
            },
        );
        assert_eq!(message.text(), "This directory is empty");
        pump_loading_deadline();
        assert!(!spinner.get_visible());

        browser.retry_column(0);
        source.latest().fail("Immediate generated failure");
        let (stack, spinner, _, message) = probe(&view);
        assert!(message.text().contains("Immediate generated failure"));
        pump_loading_deadline();
        assert_page(
            &stack,
            if mode == BrowserMode::Columns {
                "feedback"
            } else {
                "status"
            },
        );
        assert!(!spinner.get_visible());

        browser.navigate(Location::local("/generated-transitions/cancelled"));
        let cancelled = source.latest();
        browser.navigate(Location::local("/generated-transitions/replacement"));
        assert!(cancelled.cancelled.get());
        let replacement = source.latest();
        let (stack, _, model, _) = probe(&view);
        cancelled.batch("wrong-location.bin");
        cancelled.finish();
        assert_page(&stack, "pending");
        assert_eq!(model.n_items(), 0);
        replacement.batch("replacement.bin");
        replacement.finish();
        assert_page(&stack, "content");
        assert_eq!(model.n_items(), 1);

        browser.descend(
            0,
            Location::local("/generated-transitions/replacement/child"),
        );
        let child = source.latest();
        let (stack, _, model, _) = probe(&view);
        assert_page(&stack, "pending");
        assert_eq!(model.n_items(), 0);
        browser.close_column(1);
        assert!(child.cancelled.get());
        child.batch("closed-child.bin");
        let (stack, spinner, model, _) = probe(&view);
        assert_page(&stack, "content");
        assert_eq!(model.n_items(), 1);
        pump_loading_deadline();
        assert_page(&stack, "content");
        assert!(!spinner.get_visible());
        browser.clear_observer();
    }
}
