use std::cell::RefCell;

use super::*;

fn fixture(total_bytes: Option<u64>) -> (Progress, Rc<RefCell<Vec<OperationEvent>>>) {
    let events = Rc::new(RefCell::new(Vec::new()));
    let output = events.clone();
    (
        Progress::new(
            OperationRequestId(81),
            Rc::new(move |event| output.borrow_mut().push(event)),
            2,
            total_bytes,
        ),
        events,
    )
}

fn sample(event: &OperationEvent) -> (usize, u64, Option<u64>) {
    match event {
        OperationEvent::TransferProgress {
            completed,
            bytes_transferred,
            total_bytes,
            ..
        } => (*completed, *bytes_transferred, *total_bytes),
        _ => panic!("expected a byte progress event"),
    }
}

#[test]
fn active_bytes_do_not_complete_the_root_or_double_count_on_commit() {
    let (progress, events) = fixture(Some(30));
    progress.update_file(7, 10, 10);
    progress.report();
    assert_eq!(sample(&events.borrow()[0]), (0, 7, Some(30)));
    progress.update_file(10, 10, 10);
    progress.finish_file(10);
    progress.finish_root(1);
    assert_eq!(
        sample(events.borrow().last().expect("completed-root sample")),
        (1, 10, Some(30))
    );
    progress.update_file(8, 20, 20);
    progress.report();
    assert_eq!(
        sample(events.borrow().last().expect("second-file sample")),
        (1, 18, Some(30))
    );
    progress.finish_file(20);
    progress.finish_root(2);
    assert_eq!(
        sample(events.borrow().last().expect("final sample")),
        (2, 30, Some(30))
    );
}

#[test]
fn retries_remain_monotonic_and_drop_the_one_pass_estimate() {
    let (progress, events) = fixture(Some(10));
    progress.update_file(8, 10, 10);
    progress.report();
    progress.abandon_file();
    progress.update_file(2, 10, 10);
    progress.report();
    progress.update_file(1, 10, 10);
    progress.report();
    assert_eq!(sample(&events.borrow()[0]), (0, 8, Some(10)));
    assert_eq!(
        sample(&events.borrow()[1]),
        (0, 8, None),
        "invalidated estimate is reported before another decision waits"
    );
    assert_eq!(sample(&events.borrow()[2]), (0, 10, None));
    assert_eq!(sample(&events.borrow()[3]), (0, 10, None));
    progress.finish_file(10);
    progress.finish_root(1);
    assert_eq!(
        sample(events.borrow().last().expect("retried root")),
        (1, 18, None)
    );
}

#[test]
fn changing_backend_sizes_invalidate_the_total_in_both_directions() {
    for (bytes, total) in [(4, 8), (4, 12), (11, 10)] {
        let (progress, events) = fixture(Some(10));
        progress.update_file(bytes, total, 10);
        progress.report();
        assert_eq!(sample(&events.borrow()[0]), (0, bytes, None));
    }
}

#[test]
fn unknown_totals_and_skips_never_turn_into_a_percentage() {
    let (progress, events) = fixture(None);
    progress.finish_file(20);
    progress.finish_root(1);
    assert_eq!(
        sample(events.borrow().last().expect("unknown folder total")),
        (1, 20, None)
    );
    let (progress, events) = fixture(Some(20));
    progress.invalidate_total();
    progress.finish_root(1);
    assert_eq!(sample(&events.borrow()[0]), (1, 0, None));
}

#[test]
fn block_callbacks_are_throttled_and_root_completion_is_immediate() {
    let (progress, events) = fixture(Some(10_000));
    progress
        .last_update
        .set(Instant::now() + Duration::from_secs(60));
    for bytes in 0..10_000 {
        progress.update_file(bytes, 10_000, 10_000);
    }
    assert!(events.borrow().is_empty());
    progress.last_update.set(Instant::now() - UPDATE_INTERVAL);
    progress.update_file(10_000, 10_000, 10_000);
    assert_eq!(events.borrow().len(), 1);
    progress.finish_file(10_000);
    progress.finish_root(1);
    assert_eq!(events.borrow().len(), 2);
    assert_eq!(sample(&events.borrow()[1]), (1, 10_000, Some(10_000)));
}

#[test]
fn huge_backend_counts_saturate_and_invalidate_an_overflowed_estimate() {
    let (progress, events) = fixture(Some(u64::MAX));
    progress.finish_file(u64::MAX);
    progress.update_file(5, 5, 5);
    progress.report();
    assert_eq!(
        sample(events.borrow().last().expect("bounded counter")),
        (0, u64::MAX, None)
    );
    progress.invalidate_total();
    progress.finish_root(1);
    assert_eq!(
        sample(events.borrow().last().expect("overflow state")),
        (1, u64::MAX, None)
    );
}

#[test]
fn rename_completion_advances_logical_bytes_without_fabricating_copy_speed() {
    let (progress, events) = fixture(Some(50));
    progress.finish_file(30);
    progress.finish_root(1);
    assert!(matches!(
        events.borrow().last(),
        Some(OperationEvent::TransferProgress {
            bytes_transferred: 30,
            bytes_copied: 0,
            ..
        })
    ));
    progress.update_file(10, 20, 20);
    progress.report();
    progress.finish_file(20);
    progress.finish_root(2);
    assert!(matches!(
        events.borrow().last(),
        Some(OperationEvent::TransferProgress {
            bytes_transferred: 50,
            bytes_copied: 10,
            ..
        })
    ));
}
