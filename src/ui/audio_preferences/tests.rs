use super::*;

#[test]
fn saved_visualizer_options_are_normalized_without_changing_the_analyzer() {
    configure(false, 64, "unknown");
    assert_eq!(
        current(),
        Options {
            spin: false,
            columns: 32,
            decay: Decay::Studio
        }
    );
    configure(true, 16, "snappy");
    assert_eq!(
        current(),
        Options {
            spin: true,
            columns: 16,
            decay: Decay::Snappy
        }
    );
    configure(true, 32, "smooth");
    assert_eq!(current().decay, Decay::Smooth);
    assert_eq!(crate::playback::SPECTRUM_BANDS, 32);
    configure(true, 32, "studio");
}

#[test]
fn falloff_profiles_preserve_studio_and_order_response_speeds() {
    assert_eq!(Decay::Studio.hold_frames(), 10);
    assert_eq!(Decay::Studio.factor(), 0.92);
    assert!(Decay::Snappy.hold_frames() < Decay::Studio.hold_frames());
    assert!(Decay::Smooth.hold_frames() > Decay::Studio.hold_frames());
    assert!(Decay::Snappy.factor() < Decay::Studio.factor());
    assert!(Decay::Smooth.factor() > Decay::Studio.factor());
    for id in ["studio", "snappy", "smooth"] {
        assert_eq!(Decay::from_id(id).id(), id);
    }
}

#[test]
fn observers_are_weak_deduplicated_and_called_only_for_changes() {
    configure(true, 32, "studio");
    let calls = Rc::new(Cell::new(0));
    let counter = calls.clone();
    let observer: Rc<dyn Fn()> = Rc::new(move || counter.set(counter.get() + 1));
    observe(&observer);
    observe(&observer);
    configure(true, 32, "studio");
    assert_eq!(calls.get(), 0);
    configure(false, 16, "snappy");
    assert_eq!(calls.get(), 1);
    drop(observer);
    configure(true, 32, "studio");
    assert_eq!(calls.get(), 1);
    OBSERVERS.with_borrow(|observers| assert!(observers.is_empty()));
}

#[test]
fn callbacks_can_reenter_configuration_and_register_without_stale_options() {
    configure(true, 32, "studio");
    let nested: Rc<dyn Fn()> = Rc::new(|| {
        let temporary: Rc<dyn Fn()> = Rc::new(|| {});
        observe(&temporary);
    });
    let retained = nested.clone();
    let first: Rc<dyn Fn()> = Rc::new(move || {
        if !current().spin {
            observe(&retained);
            configure(true, 16, "smooth");
        }
    });
    let seen = Rc::new(Cell::new(current()));
    let value = seen.clone();
    let second: Rc<dyn Fn()> = Rc::new(move || value.set(current()));
    observe(&first);
    observe(&second);
    configure(false, 32, "snappy");
    assert_eq!(seen.get(), current());
    assert_eq!(
        current(),
        Options {
            spin: true,
            columns: 16,
            decay: Decay::Smooth
        }
    );
    drop((first, second, nested));
    configure(true, 32, "studio");
}
