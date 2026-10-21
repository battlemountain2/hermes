use super::*;

fn pump_for(duration: Duration) {
    let deadline = std::time::Instant::now() + duration;
    while std::time::Instant::now() < deadline {
        for _ in 0..64 {
            if !glib::MainContext::default().iteration(false) {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn fixture() -> (gtk::Stack, gtk::Spinner, LoadingTransition) {
    let stack = gtk::Stack::new();
    for name in ["content", "loading", "status"] {
        stack.add_named(&gtk::Label::new(Some(name)), Some(name));
    }
    let spinner = gtk::Spinner::new();
    let transition = LoadingTransition::new(&stack, &spinner);
    (stack, spinner, transition)
}

#[test]
#[ignore = "Requires isolated GTK display; delayed loading lifecycle and reduced motion"]
fn desktop_loading_transition_cancels_fast_stale_and_destroyed_feedback() {
    gtk::init().unwrap();
    let (stack, spinner, transition) = fixture();
    assert_eq!(stack.visible_child_name().as_deref(), Some("pending"));
    assert!(!spinner.get_visible());
    assert_eq!(stack.transition_type(), gtk::StackTransitionType::None);
    transition.show("content");
    pump_for(LOADING_DELAY + Duration::from_millis(40));
    assert_eq!(stack.visible_child_name().as_deref(), Some("content"));
    assert!(!spinner.is_spinning());

    transition.begin();
    let previous_generation = transition.0.generation.get();
    transition.begin();
    assert_ne!(transition.0.generation.get(), previous_generation);
    assert!(transition.0.pending.borrow().is_some());
    pump_for(LOADING_DELAY + Duration::from_millis(40));
    assert_eq!(stack.visible_child_name().as_deref(), Some("loading"));
    assert!(spinner.get_visible());
    assert!(transition.0.pending.borrow().is_none());
    transition.show("status");
    assert!(!spinner.get_visible());
    assert!(!spinner.is_spinning());

    transition.begin();
    let state = Rc::downgrade(&transition.0);
    drop(transition);
    assert!(state.upgrade().is_none());
    pump_for(LOADING_DELAY + Duration::from_millis(40));
    assert_eq!(stack.visible_child_name().as_deref(), Some("pending"));
    assert!(!spinner.get_visible());

    super::super::super::motion::set_reduce_motion(true);
    let (reduced_stack, reduced_spinner, reduced) = fixture();
    pump_for(LOADING_DELAY + Duration::from_millis(40));
    assert_eq!(
        reduced_stack.visible_child_name().as_deref(),
        Some("loading")
    );
    assert!(reduced_spinner.get_visible());
    assert!(!reduced_spinner.is_spinning());
    reduced.show("content");
    assert_eq!(
        reduced_stack.transition_type(),
        gtk::StackTransitionType::None
    );
    super::super::super::motion::set_reduce_motion(false);

    let (orphan_stack, orphan_spinner, orphan) = fixture();
    let weak_stack = orphan_stack.downgrade();
    let weak_spinner = orphan_spinner.downgrade();
    drop(orphan_stack);
    drop(orphan_spinner);
    assert!(weak_stack.upgrade().is_none());
    assert!(weak_spinner.upgrade().is_none());
    pump_for(LOADING_DELAY + Duration::from_millis(40));
    assert!(orphan.0.pending.borrow().is_none());
}
