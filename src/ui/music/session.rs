use super::*;

impl Controller {
    pub(super) fn restore_session(&self, path: PathBuf) {
        let weak = self.weak_controller();
        glib::MainContext::default().spawn_local(async move {
            let result = gtk::gio::spawn_blocking(move || crate::music::session::load(&path)).await;
            let Some(controller) = weak.upgrade() else {
                return;
            };
            if controller.closed.get() || controller.session_revision.get() != 0 {
                return;
            }
            match result {
                Ok(Ok(Some(session))) => {
                    controller.resume_position.set(session.position);
                    controller.volume_value.set(session.volume);
                    controller.restore_queue(session.into_queue());
                    let count = controller.queue.borrow().tracks.len();
                    if count > 0 {
                        controller.notice.replace(format!(
                            "Restored {count} queued songs · Resume queue when you're ready"
                        ));
                    }
                    controller.notify();
                }
                Ok(Ok(None)) => {}
                Ok(Err(error)) => {
                    tracing::warn!(%error, "Could not restore music queue");
                    controller.notice.replace(
                        "Saved queue could not be restored; your music files are unchanged".into(),
                    );
                    controller.notify();
                }
                Err(_) => tracing::warn!("Queue restore worker failed"),
            }
        });
    }

    pub(super) fn queue_changed(&self) {
        self.update_identities();
        self.queue_revision
            .set(self.queue_revision.get().wrapping_add(1));
        self.changed_session();
        self.notify();
    }

    pub(super) fn changed_session(&self) {
        if self.restoring.get() || self.closed.get() {
            return;
        }
        self.session_revision
            .set(self.session_revision.get().wrapping_add(1));
        self.save_session();
    }

    pub(super) fn save_session(&self) {
        if self.session_revision.get() == 0 {
            return;
        }
        let session = crate::music::session::Session::capture(
            &self.queue.borrow(),
            self.position(),
            self.volume(),
        );
        if let Some(store) = self.session_store.borrow().as_ref() {
            store.save(session);
        }
    }

    pub(super) fn report_save_error(&self) {
        let error = self
            .session_store
            .borrow()
            .as_ref()
            .and_then(crate::music::session::Store::take_error);
        if let Some(error) = error {
            tracing::warn!(%error, "Could not save music queue");
            self.notice.replace(format!(
                "Queue changes work, but could not be saved: {error}"
            ));
            self.notify();
        }
    }
}
