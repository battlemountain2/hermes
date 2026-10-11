use super::*;
use gtk::subclass::prelude::*;
use std::cell::RefCell;

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct TestStream {
        pub seeks: RefCell<Vec<i64>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for TestStream {
        const NAME: &'static str = "HermesSeekTestStream";
        type Type = super::TestStream;
        type ParentType = gtk::MediaStream;
        type Interfaces = (gdk::Paintable,);
    }

    impl ObjectImpl for TestStream {}
    impl MediaStreamImpl for TestStream {
        fn seek(&self, timestamp: i64) {
            self.seeks.borrow_mut().push(timestamp);
        }
    }
    impl gdk::subclass::prelude::PaintableImpl for TestStream {
        fn snapshot(&self, _: &gdk::Snapshot, _: f64, _: f64) {}
    }
}

glib::wrapper! {
    pub struct TestStream(ObjectSubclass<imp::TestStream>)
        @extends gtk::MediaStream, @implements gdk::Paintable;
}

fn stream() -> TestStream {
    let stream: TestStream = glib::Object::new();
    stream.stream_prepared(true, false, true, 60_000_000);
    stream
}

#[test]
fn timestamps_are_bounded_and_finite() {
    assert_eq!(timestamp(3.5, 30_000_000), 3_500_000);
    assert_eq!(timestamp(45.0, 30_000_000), 29_999_999);
    assert_eq!(timestamp(-2.0, 30_000_000), 0);
    assert_eq!(timestamp(f64::NAN, 30_000_000), 0);
    assert_eq!(timestamp(f64::INFINITY, 30_000_000), 0);
    assert_eq!(timestamp(3.0, -1), 0);
}

#[test]
#[ignore = "Requires a GTK display; deterministic stream, no personal media"]
fn desktop_drag_preserves_thumb_and_coalesces_pending_seeks() {
    gtk::init().unwrap();
    let first = stream();
    let active = Rc::new(RefCell::new(Some(
        first.clone().upcast::<gtk::MediaStream>(),
    )));
    let scale = gtk::Scale::with_range(gtk::Orientation::Horizontal, 0.0, 60.0, 0.1);
    let source = active.clone();
    let target = active.clone();
    let control = SeekControl::new(
        &scale,
        move || source.borrow().clone(),
        move |timestamp| {
            if let Some(stream) = target.borrow().clone() {
                stream.seek(timestamp);
            }
        },
    );
    let weak = Rc::downgrade(&control);
    first.connect_notify_local(None, move |_, _| {
        if let Some(control) = weak.upgrade() {
            control.sync();
        }
    });
    first.update(2_000_000);
    assert_eq!(scale.value(), 2.0);
    assert!(first.imp().seeks.borrow().is_empty());
    control.begin();
    scale.set_value(10.0);
    scale.set_value(20.0);
    first.update(3_000_000);
    assert_eq!(scale.value(), 20.0);
    assert!(first.imp().seeks.borrow().is_empty());
    assert!(scale.is_sensitive());
    control.finish();
    while glib::MainContext::default().iteration(false) {}
    assert_eq!(*first.imp().seeks.borrow(), [20_000_000]);
    assert!(first.is_seeking());
    assert!(
        scale.is_sensitive(),
        "Do not break GTK's pointer grab mid-seek"
    );
    scale.set_value(30.0);
    scale.set_value(40.0);
    first.update(20_000_000);
    assert_eq!(scale.value(), 40.0);
    assert_eq!(first.imp().seeks.borrow().len(), 1);
    first.seek_success();
    assert_eq!(*first.imp().seeks.borrow(), [20_000_000, 40_000_000]);
    first.update(40_000_000);
    first.seek_success();
    assert_eq!(scale.value(), 40.0);
    assert_eq!(control.pending.get(), None);
    control.begin();
    scale.set_value(50.0);
    control.cancel();
    control.finish();
    while glib::MainContext::default().iteration(false) {}
    assert_eq!(first.imp().seeks.borrow().len(), 2);
    control.sync();
    assert_eq!(scale.value(), 40.0);
    control.begin();
    scale.set_value(55.0);
    let second = stream();
    active.replace(Some(second.clone().upcast()));
    control.sync();
    control.finish();
    while glib::MainContext::default().iteration(false) {}
    assert_eq!(scale.value(), 0.0);
    assert!(second.imp().seeks.borrow().is_empty());
    active.replace(None);
    control.sync();
    assert!(!scale.is_sensitive());
    assert_eq!(control.pending.get(), None);
    let weak = Rc::downgrade(&control);
    drop(control);
    drop(scale);
    assert!(weak.upgrade().is_none());
}
