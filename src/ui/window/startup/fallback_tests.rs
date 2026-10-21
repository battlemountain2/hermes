use super::*;

use crate::services::{
    DirectoryEvent, DirectoryRequest, FileSource, LoadHandle, LocationValidationError,
    StoredTrails, TrailStore,
};

#[derive(Default)]
struct MemoryStore(RefCell<StoredTrails>);

impl TrailStore for MemoryStore {
    fn load(&self) -> std::io::Result<StoredTrails> {
        Ok(self.0.borrow().clone())
    }

    fn save(&self, stored: &StoredTrails) -> std::io::Result<()> {
        self.0.replace(stored.clone());
        Ok(())
    }
}

#[derive(Default)]
struct DeferredSource {
    requests: RefCell<Vec<(DirectoryRequest, Rc<dyn Fn(DirectoryEvent)>)>>,
}

impl FileSource for DeferredSource {
    fn validate_location(&self, _: &Location) -> Result<(), LocationValidationError> {
        Ok(())
    }

    fn enumerate(&self, request: DirectoryRequest, emit: Rc<dyn Fn(DirectoryEvent)>) -> LoadHandle {
        self.requests.borrow_mut().push((request, emit));
        LoadHandle::new(|| {})
    }
}

#[test]
#[ignore = "Run alone with an isolated GLib main context; synthetic source and store only"]
fn startup_failure_fallback_is_deferred_and_cannot_override_manual_navigation() {
    let context = glib::MainContext::default();
    let _guard = context.acquire().unwrap();
    for interruption in ["none", "navigate", "return", "tab", "dispose"] {
        let home = Location::local("/startup-fixture/home");
        let offline = Location::local("/startup-fixture/offline");
        let trails = Trails::load(Rc::new(MemoryStore::default())).unwrap();
        let view = fresh_view(
            TrailBrowserMode::Columns,
            TrailBrowserDensity::Compact,
            false,
        );
        let failed_id = trails
            .create("Offline", offline.clone(), view.clone())
            .unwrap();
        let selected = select(&trails, None, "last", &home, &view);
        let restore = Rc::new(RefCell::new(Restore::new(&selected.trail)));
        let source = Rc::new(DeferredSource::default());
        let browser = Browser::new(source.clone());
        let activated = Rc::new(RefCell::new(Vec::new()));
        let recorded = activated.clone();
        let weak_browser = Rc::downgrade(&browser);
        install_fallback(
            &browser,
            trails.clone(),
            restore,
            home.clone(),
            view.clone(),
            Rc::new(move |trail| {
                recorded.borrow_mut().push(trail.clone());
                if let Some(browser) = weak_browser.upgrade() {
                    let location = trail.active_location().unwrap().clone();
                    browser.activate_tab(trail.id, location, trail.view.preferences);
                }
            }),
        );
        browser.activate_tab(selected.trail.id, offline.clone(), view.preferences);
        let (request, emit) = source.requests.borrow()[0].clone();
        emit(DirectoryEvent::Failed {
            request_id: request.id,
            message: "Generated unavailable destination".into(),
        });
        assert!(activated.borrow().is_empty());
        assert_eq!(browser.active_location(), Some(offline.clone()));
        match interruption {
            "navigate" => browser.navigate(Location::local("/startup-fixture/manual")),
            "return" => {
                browser.navigate(Location::local("/startup-fixture/manual"));
                browser.navigate(offline.clone());
            }
            "dispose" => browser.clear_observer(),
            "tab" => {
                trails
                    .create("Another tab", offline.clone(), view.clone())
                    .unwrap();
            }
            _ => {}
        }
        for _ in 0..64 {
            if !context.iteration(false) {
                break;
            }
        }
        if interruption == "none" {
            assert_eq!(activated.borrow().len(), 1);
            assert_eq!(browser.active_location(), Some(home));
        } else {
            assert!(activated.borrow().is_empty());
            if interruption == "dispose" {
                assert_eq!(trails.all().len(), 1);
            }
            if interruption == "navigate" {
                assert_eq!(
                    browser.active_location(),
                    Some(Location::local("/startup-fixture/manual"))
                );
            }
        }
        assert_eq!(
            trails
                .all()
                .into_iter()
                .find(|trail| trail.id == failed_id)
                .unwrap()
                .active_location(),
            Some(&offline)
        );
        browser.clear_observer();
    }
}
