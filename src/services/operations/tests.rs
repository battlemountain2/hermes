// SPDX-License-Identifier: GPL-3.0-or-later

use super::{ConflictAction, ConflictReply, ConflictResolution, validate_basename};

#[test]
fn conflict_reply_wakes_the_waiter_and_keeps_the_first_decision() {
    use std::{
        future::Future,
        pin::pin,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        task::{Context, Poll, Wake, Waker},
    };
    struct Counter(AtomicUsize);
    impl Wake for Counter {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
    let reply = ConflictReply::default();
    let wakes = Arc::new(Counter(AtomicUsize::new(0)));
    let waker = Waker::from(wakes.clone());
    let mut context = Context::from_waker(&waker);
    let mut future = pin!(reply.wait());
    assert!(future.as_mut().poll(&mut context).is_pending());
    let chosen = ConflictResolution {
        action: ConflictAction::KeepBoth,
        apply_to_remaining: false,
    };
    reply.respond(chosen);
    reply.respond(ConflictResolution {
        action: ConflictAction::Cancel,
        apply_to_remaining: true,
    });
    assert_eq!(wakes.0.load(Ordering::Relaxed), 1);
    assert_eq!(future.as_mut().poll(&mut context), Poll::Ready(chosen));
}

#[test]
fn basenames_reject_empty_reserved_nested_absolute_and_nul_names() {
    for name in [
        "",
        ".",
        "..",
        "../escaped",
        "nested/child",
        "/tmp/absolute",
        "nul\0name",
    ] {
        assert!(
            validate_basename(name).is_err(),
            "{name:?} should be rejected"
        );
    }
}

#[test]
fn basenames_accept_single_native_and_unicode_components() {
    for name in ["report.txt", "folder name", ".config", "résumé"] {
        assert!(
            validate_basename(name).is_ok(),
            "{name:?} should be accepted"
        );
    }
}
