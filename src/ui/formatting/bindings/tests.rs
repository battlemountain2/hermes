use std::cell::Cell;
use std::time::Instant;

use super::super::{OPTIONS, Options, SizeUnit, configure};
use super::*;

struct Fixture {
    options: Options,
    windows: Vec<gtk::Window>,
}

impl Fixture {
    fn new() -> Self {
        gtk::init().unwrap();
        let options = OPTIONS.get();
        configure("relative", "decimal");
        Self {
            options,
            windows: Vec::new(),
        }
    }

    fn show(&mut self, child: &impl IsA<gtk::Widget>) {
        let window = gtk::Window::builder()
            .default_width(320)
            .default_height(120)
            .child(child)
            .build();
        self.windows.push(window.clone());
        window.present();
        wait_for(|| child.is_mapped());
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        for window in self.windows.drain(..) {
            window.set_child(None::<&gtk::Widget>);
            window.destroy();
        }
        configure(
            match self.options.date {
                DateFormat::Relative => "relative",
                DateFormat::Iso => "iso",
            },
            match self.options.size {
                SizeUnit::Decimal => "decimal",
                SizeUnit::Binary => "binary",
            },
        );
    }
}

fn wait_for(predicate: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while !predicate() && Instant::now() < deadline {
        glib::MainContext::default().iteration(false);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(
        predicate(),
        "isolated GTK fixture did not reach its expected state"
    );
}

fn snapshot(label: &gtk::Label) -> Update {
    REGISTRY.with_borrow(|registry| registry.update(label.as_ptr() as usize).unwrap())
}

fn timer_identity() -> Rc<()> {
    REGISTRY.with_borrow(|registry| registry.timer.as_ref().unwrap().identity.clone())
}

fn has_timer() -> bool {
    REGISTRY.with_borrow(|registry| registry.timer.is_some())
}

#[test]
#[ignore = "Requires an isolated GTK display; in-memory formatting only"]
fn live_refresh_updates_retained_labels_and_absolute_date_tooltips() {
    let _fixture = Fixture::new();
    let seconds = glib::DateTime::now_local().unwrap().to_unix() - 310;
    let values = [
        Value::Size(1_048_576),
        Value::Timestamp(seconds),
        Value::Selection(3, 1_048_576),
        Value::FreeSpace(1_048_576),
    ];
    let labels: Vec<_> = values
        .iter()
        .map(|&value| {
            let label = gtk::Label::new(None);
            bind(&label, value);
            label
        })
        .collect();
    labels[0].set_tooltip_text(Some("Custom size explanation"));
    let relative = labels[1].text();
    assert_eq!(
        labels[1].tooltip_text().as_deref(),
        Some(format_absolute(seconds).as_str())
    );
    assert!(!has_timer());

    configure("iso", "binary");
    for (label, value) in labels.iter().zip(values) {
        assert_eq!(label.text(), format_value(value));
        assert!(!label.is_mapped());
    }
    assert_ne!(labels[1].text(), relative);
    assert_eq!(
        labels[0].tooltip_text().as_deref(),
        Some("Custom size explanation")
    );
    assert_eq!(
        labels[1].tooltip_text().as_deref(),
        Some(format_absolute(seconds).as_str())
    );

    let notifications = Rc::new(Cell::new(0));
    for label in &labels {
        let notifications = notifications.clone();
        label.connect_label_notify(move |_| notifications.set(notifications.get() + 1));
    }
    let epoch = REGISTRY.with_borrow(|registry| registry.epoch.clone());
    configure("iso", "binary");
    assert_eq!(notifications.get(), 0);
    assert!(REGISTRY.with_borrow(|registry| Rc::ptr_eq(&epoch, &registry.epoch)));
    configure("relative", "decimal");
    assert_eq!(labels[1].text(), format_value(Value::Timestamp(seconds)));
    assert_eq!(notifications.get(), 4);
}

#[test]
#[ignore = "Requires an isolated GTK display; in-memory formatting only"]
fn recycled_label_retargets_and_clears_without_duplicating_ownership() {
    let _fixture = Fixture::new();
    let label = gtk::Label::new(None);
    let seconds = 1_700_000_000;
    bind(&label, Value::Timestamp(seconds));
    let identity = snapshot(&label).identity;
    for _ in 0..16 {
        bind(&label, Value::Timestamp(seconds));
        assert!(label.tooltip_text().is_some());
        bind(&label, Value::Size(2048));
        assert!(label.tooltip_text().is_none());
        label.set_tooltip_text(Some("Keep this size tooltip"));
        bind(&label, Value::Selection(2, 2048));
        bind(&label, Value::FreeSpace(2048));
        assert_eq!(
            label.tooltip_text().as_deref(),
            Some("Keep this size tooltip")
        );
        clear(&label, "Directory");
        assert_eq!(label.text(), "Directory");
        assert_eq!(
            label.tooltip_text().as_deref(),
            Some("Keep this size tooltip")
        );
        assert!(Rc::ptr_eq(&identity, &snapshot(&label).identity));
    }
    bind(&label, Value::Timestamp(seconds));
    clear(&label, "Unavailable");
    assert!(label.tooltip_text().is_none());
    let stale = snapshot(&label);
    bind(&label, Value::Size(2048));
    label.set_tooltip_text(Some("Replacement tooltip"));
    let notifications = Rc::new(Cell::new(0));
    let observed = notifications.clone();
    label.connect_notify_local(None, move |_, _| observed.set(observed.get() + 1));
    render(stale, "Stale fallback");
    destroyed(label.as_ptr() as usize, &Rc::new(()));
    refresh();
    assert_eq!(notifications.get(), 0);
    assert_eq!(label.text(), format_value(Value::Size(2048)));
    assert_eq!(label.tooltip_text().as_deref(), Some("Replacement tooltip"));
    assert!(Rc::ptr_eq(&identity, &snapshot(&label).identity));
}

#[test]
#[ignore = "Requires an isolated GTK display; bounded synthetic window, no media or settings IO"]
fn mapped_relative_dates_share_one_minute_timer_and_resume_on_map() {
    let mut fixture = Fixture::new();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let first = gtk::Label::new(None);
    let second = gtk::Label::new(None);
    let size = gtk::Label::new(None);
    for label in [&first, &second, &size] {
        content.append(label);
    }
    let seconds = glib::DateTime::now_local().unwrap().to_unix() - 310;
    bind(&first, Value::Timestamp(seconds));
    bind(&second, Value::Timestamp(seconds));
    bind(&size, Value::Size(2048));
    assert!(!has_timer());
    fixture.show(&content);
    wait_for(|| first.is_mapped() && second.is_mapped());
    let original_timer = timer_identity();
    REGISTRY.with_borrow(|registry| {
        assert_eq!(registry.mapped_dates.len(), 2);
        let source = glib::MainContext::default()
            .find_source_by_id(&registry.timer.as_ref().unwrap().source)
            .unwrap();
        let remaining = source.ready_time() - glib::monotonic_time();
        assert!((55_000_000..=60_000_000).contains(&remaining));
    });
    for _ in 0..16 {
        bind(&first, Value::Timestamp(seconds));
        clear(&first, "Recycled");
        bind(&first, Value::Timestamp(seconds));
    }
    configure("relative", "binary");
    assert!(Rc::ptr_eq(&original_timer, &timer_identity()));
    first.set_visible(false);
    wait_for(|| !first.is_mapped());
    assert!(Rc::ptr_eq(&original_timer, &timer_identity()));
    first.set_text("Hidden stale text");
    second.set_text("Mapped stale text");
    assert_eq!(tick(&original_timer), glib::ControlFlow::Continue);
    assert_eq!(first.text(), "Hidden stale text");
    assert_eq!(second.text(), format_value(Value::Timestamp(seconds)));
    first.set_visible(true);
    wait_for(|| first.is_mapped());
    assert_eq!(first.text(), format_value(Value::Timestamp(seconds)));
    configure("iso", "binary");
    assert!(!has_timer());
    assert_eq!(tick(&original_timer), glib::ControlFlow::Break);
    configure("relative", "binary");
    assert!(!Rc::ptr_eq(&original_timer, &timer_identity()));
    first.set_visible(false);
    second.set_visible(false);
    wait_for(|| !first.is_mapped() && !second.is_mapped());
    assert!(!has_timer());
    second.set_visible(true);
    wait_for(|| second.is_mapped());
    assert!(has_timer());
    clear(&second, "No date");
    assert!(!has_timer());
    bind(&second, Value::Timestamp(seconds));
    assert!(has_timer());
    bind(&second, Value::Size(2048));
    assert!(!has_timer());
}

#[test]
#[ignore = "Requires an isolated GTK display; in-memory formatting only"]
fn weak_destruction_removes_registry_entries_and_discards_stale_updates() {
    let _fixture = Fixture::new();
    let label = gtk::Label::new(None);
    bind(&label, Value::Timestamp(1_700_000_000));
    let stale = snapshot(&label);
    let key = label.as_ptr() as usize;
    let weak = label.downgrade();
    let notifications = Rc::new(Cell::new(0));
    let observed = notifications.clone();
    label.connect_notify_local(None, move |_, _| observed.set(observed.get() + 1));
    drop(label);
    assert!(weak.upgrade().is_none());
    assert!(REGISTRY.with_borrow(|registry| !registry.labels.contains_key(&key)));
    let before = notifications.get();
    render(stale, "Stale");
    configure("iso", "binary");
    assert_eq!(notifications.get(), before);
    assert!(!has_timer());
}

#[test]
#[ignore = "Requires an isolated GTK display; bounded synthetic window, no media or settings IO"]
fn destroying_last_mapped_date_releases_widget_and_timer() {
    let mut fixture = Fixture::new();
    let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let label = gtk::Label::new(None);
    content.append(&label);
    bind(&label, Value::Timestamp(1_700_000_000));
    fixture.show(&content);
    wait_for(|| label.is_mapped());
    let timer = timer_identity();
    let weak = label.downgrade();
    let key = label.as_ptr() as usize;
    content.remove(&label);
    drop(label);
    assert!(weak.upgrade().is_none());
    assert!(REGISTRY.with_borrow(|registry| !registry.labels.contains_key(&key)));
    assert!(!has_timer());
    assert_eq!(tick(&timer), glib::ControlFlow::Break);
}

#[test]
#[ignore = "Requires an isolated GTK display; in-memory formatting only"]
fn reentrant_tooltip_rebind_cannot_publish_stale_date_text() {
    let _fixture = Fixture::new();
    let label = gtk::Label::new(None);
    let rebound = Cell::new(false);
    label.connect_tooltip_text_notify(move |label| {
        if !rebound.replace(true) {
            bind(label, Value::Size(2048));
        }
    });
    let notifications = Rc::new(Cell::new(0));
    let observed = notifications.clone();
    label.connect_label_notify(move |_| observed.set(observed.get() + 1));
    bind(&label, Value::Timestamp(1_700_000_000));
    assert_eq!(label.text(), format_value(Value::Size(2048)));
    assert!(label.tooltip_text().is_none());
    assert_eq!(notifications.get(), 1);
}

#[test]
#[ignore = "Requires an isolated GTK display; in-memory formatting only"]
fn reentrant_preferences_and_clear_keep_only_the_latest_rendering() {
    let _fixture = Fixture::new();
    let label = gtk::Label::new(None);
    let configured = Cell::new(false);
    let handler = label.connect_tooltip_text_notify(move |_| {
        if !configured.replace(true) {
            configure("iso", "binary");
        }
    });
    let seconds = glib::DateTime::now_local().unwrap().to_unix() - 310;
    bind(&label, Value::Timestamp(seconds));
    assert_eq!(label.text(), format_value(Value::Timestamp(seconds)));
    assert_eq!(date_format(), DateFormat::Iso);
    label.disconnect(handler);

    let stale = snapshot(&label);
    label.connect_tooltip_text_notify(move |_| configure("relative", "decimal"));
    clear(&label, "No timestamp");
    assert_eq!(label.text(), "No timestamp");
    assert!(label.tooltip_text().is_none());
    assert_eq!(date_format(), DateFormat::Relative);
    let notifications = Rc::new(Cell::new(0));
    let observed = notifications.clone();
    label.connect_notify_local(None, move |_, _| observed.set(observed.get() + 1));
    render(stale, "");
    configure("iso", "binary");
    assert_eq!(label.text(), "No timestamp");
    assert_eq!(notifications.get(), 0);
}

#[test]
#[ignore = "Requires an isolated GTK display; in-memory formatting only"]
fn reentrant_text_notification_can_clear_the_current_date_binding() {
    let _fixture = Fixture::new();
    let label = gtk::Label::new(None);
    let cleared = Cell::new(false);
    label.connect_label_notify(move |label| {
        if !cleared.replace(true) {
            clear(label, "Recycled row");
        }
    });
    bind(&label, Value::Timestamp(1_700_000_000));
    assert_eq!(label.text(), "Recycled row");
    assert!(label.tooltip_text().is_none());
    configure("iso", "binary");
    assert_eq!(label.text(), "Recycled row");
    assert!(!has_timer());
}
