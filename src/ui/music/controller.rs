use super::*;
use std::rc::Weak;

thread_local! {
    static CONTROLLERS: RefCell<Vec<(glib::WeakRef<gtk::Application>, Weak<Controller>)>> = const { RefCell::new(Vec::new()) };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PlaybackStatus {
    Stopped,
    Paused,
    Playing,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct Capabilities {
    pub can_next: bool,
    pub can_previous: bool,
    pub can_play: bool,
    pub can_pause: bool,
    pub can_seek: bool,
}

#[derive(Default)]
struct TrackIdentity {
    version: u64,
    next: u64,
    occurrences: Vec<u64>,
}

impl TrackIdentity {
    fn replace(&mut self, count: usize) {
        self.version += 1;
        self.occurrences.clear();
        self.extend(count);
    }

    fn extend(&mut self, count: usize) {
        while self.occurrences.len() < count {
            self.next += 1;
            self.occurrences.push(self.next);
        }
    }

    fn remove(&mut self, index: usize) {
        if index < self.occurrences.len() {
            self.occurrences.remove(index);
        }
    }

    fn current(&self, index: Option<usize>) -> Option<(u64, u64)> {
        Some((self.version, *self.occurrences.get(index?)?))
    }
}

pub(crate) struct Controller {
    weak: Weak<Self>,
    pub(super) stream: RefCell<Option<PlaybackStream>>,
    pub(super) queue: RefCell<Queue>,
    pub(super) generation: Cell<u64>,
    pub(super) peaks: RefCell<Vec<f32>>,
    pub(super) folder_cancel: RefCell<Cancellation>,
    waveform_cancel: RefCell<Cancellation>,
    pub(super) queue_revision: Cell<u64>,
    pub(super) drag_identity: u64,
    pub(super) session_store: RefCell<Option<crate::music::session::Store>>,
    pub(super) session_revision: Cell<u64>,
    pub(super) restoring: Cell<bool>,
    pub(super) resume_position: Cell<i64>,
    pub(super) play_intent: Cell<bool>,
    pub(super) closed: Cell<bool>,
    pub(super) volume_value: Cell<f64>,
    pub(super) transport_visible: Cell<bool>,
    pub(super) notice: RefCell<String>,
    pub(super) waveform_notice: RefCell<String>,
    identities: RefCell<TrackIdentity>,
    views: RefCell<Vec<Weak<State>>>,
    observers: RefCell<Vec<Weak<dyn Fn()>>>,
    notifying: Cell<bool>,
    pending_notify: Cell<bool>,
    seek_revision: Cell<u64>,
    checkpoint: Cell<std::time::Instant>,
}

impl Controller {
    pub(super) fn weak_controller(&self) -> Weak<Self> {
        self.weak.clone()
    }

    pub(super) fn update_identities(&self) {
        self.identities
            .borrow_mut()
            .extend(self.queue.borrow().tracks.len());
    }

    pub(super) fn restore_queue(&self, queue: Queue) {
        self.identities.borrow_mut().replace(queue.tracks.len());
        self.queue.replace(queue);
        self.queue_revision
            .set(self.queue_revision.get().wrapping_add(1));
    }

    pub(crate) fn for_application(application: &gtk::Application) -> Rc<Self> {
        Self::for_application_with_session(application, Some(crate::music::session::path()))
    }

    fn for_application_with_session(
        application: &gtk::Application,
        session_path: Option<PathBuf>,
    ) -> Rc<Self> {
        if let Some(controller) = CONTROLLERS.with(|registry| {
            let mut registry = registry.borrow_mut();
            registry.retain(|(app, controller)| {
                app.upgrade().is_some()
                    && controller
                        .upgrade()
                        .is_some_and(|controller| !controller.closed.get())
            });
            registry.iter().find_map(|(app, controller)| {
                (app.upgrade().as_ref() == Some(application))
                    .then(|| controller.upgrade())
                    .flatten()
                    .filter(|controller| !controller.closed.get())
            })
        }) {
            return controller;
        }
        let controller = Self::with_session(session_path);
        CONTROLLERS.with(|registry| {
            registry
                .borrow_mut()
                .push((application.downgrade(), Rc::downgrade(&controller)));
        });
        let retained = controller.clone();
        application.connect_shutdown(move |_| {
            retained.shutdown();
            CONTROLLERS.with(|registry| {
                registry.borrow_mut().retain(|(_, controller)| {
                    controller
                        .upgrade()
                        .is_some_and(|controller| !Rc::ptr_eq(&controller, &retained))
                });
            });
        });
        controller
    }

    fn new(session_path: Option<&std::path::Path>) -> Rc<Self> {
        let controller = Rc::new_cyclic(|weak| Self {
            weak: weak.clone(),
            stream: RefCell::new(None),
            queue: RefCell::new(Queue::default()),
            generation: Cell::new(0),
            peaks: RefCell::new(Vec::new()),
            folder_cancel: RefCell::new(Cancellation::default()),
            waveform_cancel: RefCell::new(Cancellation::default()),
            queue_revision: Cell::new(0),
            drag_identity: {
                static NEXT_CONTROLLER: std::sync::atomic::AtomicU64 =
                    std::sync::atomic::AtomicU64::new(1);
                NEXT_CONTROLLER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            },
            session_store: RefCell::new(
                session_path
                    .as_ref()
                    .map(|path| crate::music::session::Store::new(path.to_path_buf())),
            ),
            session_revision: Cell::new(0),
            restoring: Cell::new(false),
            resume_position: Cell::new(0),
            play_intent: Cell::new(false),
            closed: Cell::new(false),
            volume_value: Cell::new(0.75),
            transport_visible: Cell::new(false),
            notice: RefCell::new(String::new()),
            waveform_notice: RefCell::new("Now playing waveform · click to seek".into()),
            identities: RefCell::new(TrackIdentity::default()),
            views: RefCell::new(Vec::new()),
            observers: RefCell::new(Vec::new()),
            notifying: Cell::new(false),
            pending_notify: Cell::new(false),
            seek_revision: Cell::new(0),
            checkpoint: Cell::new(std::time::Instant::now()),
        });
        controller
    }

    pub(crate) fn with_session(session_path: Option<PathBuf>) -> Rc<Self> {
        let controller = Self::new(session_path.as_deref());
        let weak = controller.weak.clone();
        glib::timeout_add_local(Duration::from_millis(100), move || {
            let Some(controller) = weak.upgrade().filter(|controller| !controller.closed.get())
            else {
                return glib::ControlFlow::Break;
            };
            if controller.checkpoint.get().elapsed() >= Duration::from_secs(10) {
                controller.checkpoint.set(std::time::Instant::now());
                if controller.stream.borrow().is_some() {
                    controller.changed_session();
                }
                controller.report_save_error();
            }
            let has_stream = controller.stream.borrow().is_some();
            if has_stream {
                controller.notify();
            }
            glib::ControlFlow::Continue
        });
        if let Some(path) = session_path {
            controller.restore_session(path);
        }
        controller
    }

    pub(super) fn attach(&self, state: &Rc<State>) {
        self.views.borrow_mut().push(Rc::downgrade(state));
        state.sync_controller();
    }

    pub(crate) fn observe(&self, callback: &Rc<dyn Fn()>) {
        self.observers.borrow_mut().push(Rc::downgrade(callback));
    }

    pub(super) fn notify(&self) {
        if self.notifying.replace(true) {
            self.pending_notify.set(true);
            return;
        }
        loop {
            self.pending_notify.set(false);
            let views: Vec<_> = {
                let mut views = self.views.borrow_mut();
                views.retain(|view| view.strong_count() > 0);
                views.iter().filter_map(Weak::upgrade).collect()
            };
            for view in views {
                if !view.disposed.get() {
                    view.sync_controller();
                }
            }
            let observers: Vec<_> = {
                let mut observers = self.observers.borrow_mut();
                observers.retain(|callback| callback.strong_count() > 0);
                observers.iter().filter_map(Weak::upgrade).collect()
            };
            for callback in observers {
                callback();
            }
            if !self.pending_notify.get() {
                break;
            }
        }
        self.notifying.set(false);
    }

    pub(crate) fn current_track(&self) -> Option<Track> {
        self.queue.borrow().playing().cloned()
    }

    pub(crate) fn track_id(&self) -> Option<(u64, u64)> {
        self.identities
            .borrow()
            .current(self.queue.borrow().current)
    }

    pub(crate) fn position(&self) -> i64 {
        self.stream
            .borrow()
            .as_ref()
            .filter(|stream| stream.is_prepared())
            .map_or(self.resume_position.get(), |stream| stream.timestamp())
            .max(0)
    }

    pub(crate) fn duration(&self) -> i64 {
        self.stream
            .borrow()
            .as_ref()
            .map_or(0, |stream| stream.duration())
            .max(0)
    }

    pub(crate) fn status(&self) -> PlaybackStatus {
        let stream = self.stream.borrow().clone();
        match stream {
            None => PlaybackStatus::Stopped,
            Some(stream) if stream.error().is_some() => PlaybackStatus::Stopped,
            Some(stream)
                if stream.is_playing() || (!stream.is_prepared() && self.play_intent.get()) =>
            {
                PlaybackStatus::Playing
            }
            Some(_) => PlaybackStatus::Paused,
        }
    }

    pub(crate) fn capabilities(&self) -> Capabilities {
        if self.closed.get() {
            return Capabilities::default();
        }
        let queue = self.queue.borrow();
        Capabilities {
            can_next: queue.can_advance(),
            can_previous: queue.current.is_some(),
            can_play: !queue.tracks.is_empty(),
            can_pause: self
                .stream
                .borrow()
                .as_ref()
                .is_some_and(|stream| stream.error().is_none()),
            can_seek: self.stream.borrow().as_ref().is_some_and(|stream| {
                stream.is_prepared() && stream.is_seekable() && stream.error().is_none()
            }),
        }
    }

    pub(crate) fn volume(&self) -> f64 {
        self.volume_value.get()
    }
    pub(crate) fn repeat(&self) -> Repeat {
        self.queue.borrow().repeat
    }
    pub(crate) fn shuffle(&self) -> bool {
        self.queue.borrow().shuffle
    }
    pub(crate) fn seek_revision(&self) -> u64 {
        self.seek_revision.get()
    }

    fn discontinuity(&self) {
        self.seek_revision.set(self.seek_revision.get() + 1);
    }

    pub(crate) fn play(&self) {
        if self.closed.get() {
            return;
        }
        let stream = self.stream.borrow().clone();
        if let Some(stream) = stream.filter(|stream| stream.error().is_none()) {
            self.play_intent.set(true);
            if let Some(track) = self.current_track() {
                self.notice
                    .replace(format!("Now playing · {}", track.title()));
            }
            if stream.is_prepared() {
                stream.play();
            }
            self.changed_session();
            self.notify();
        } else {
            let track = {
                let mut queue = self.queue.borrow_mut();
                queue.playing().cloned().or_else(|| queue.advance(false))
            };
            if let Some(track) = track {
                self.start(track);
            }
        }
    }

    pub(crate) fn pause(&self) {
        if self.closed.get() {
            return;
        }
        self.play_intent.set(false);
        let stream = self.stream.borrow().clone();
        if let Some(stream) = stream {
            stream.pause();
            if let Some(track) = self.current_track() {
                self.notice.replace(format!("Paused · {}", track.title()));
            }
        }
        self.changed_session();
        self.notify();
    }

    pub(crate) fn toggle(&self) {
        if self.play_intent.get() && self.status() == PlaybackStatus::Playing {
            self.pause();
        } else {
            self.play();
        }
    }

    pub(crate) fn stop(&self) {
        if self.closed.get() {
            return;
        }
        self.release_stream();
        self.resume_position.set(0);
        self.notice.replace("Stopped".into());
        self.discontinuity();
        self.queue_changed();
    }

    fn release_stream(&self) {
        self.play_intent.set(false);
        self.transport_visible.set(false);
        self.generation.set(self.generation.get().wrapping_add(1));
        let stream = self.stream.borrow_mut().take();
        if let Some(stream) = stream {
            stream.stop();
        }
        self.waveform_cancel.borrow().cancel();
        self.folder_cancel.borrow().cancel();
        self.peaks.borrow_mut().clear();
    }

    pub(crate) fn next(&self) {
        if self.closed.get() {
            return;
        }
        let next = self.queue.borrow_mut().advance(false);
        self.navigate(next);
    }

    pub(crate) fn previous(&self) {
        if self.closed.get() {
            return;
        }
        if self.position() > 3_000_000 {
            if self.stream.borrow().is_some() {
                self.seek(0);
            } else {
                self.resume_position.set(0);
                self.discontinuity();
                self.changed_session();
                self.notify();
            }
            return;
        }
        let previous = self.queue.borrow_mut().previous();
        if let Some(track) = previous {
            self.navigate(Some(track));
        }
    }

    fn navigate(&self, track: Option<Track>) {
        let status = self.status();
        self.resume_position.set(0);
        if let Some(track) = track {
            match status {
                PlaybackStatus::Playing => self.start_with(track, true),
                PlaybackStatus::Paused => self.start_with(track, false),
                PlaybackStatus::Stopped => {
                    self.release_stream();
                    self.notice.replace(format!("Stopped · {}", track.title()));
                    self.discontinuity();
                    self.queue_changed();
                }
            }
        } else {
            self.stop();
        }
    }

    pub(super) fn start_or_finish(&self, next: Option<Track>) {
        if let Some(track) = next {
            self.start_with(track, self.play_intent.get());
        } else {
            self.stop();
            self.notice.replace("Queue finished".into());
            self.notify();
        }
    }

    pub(crate) fn seek(&self, position: i64) {
        if self.closed.get() || position < 0 {
            return;
        }
        let stream = self.stream.borrow().clone();
        if let Some(stream) = stream.filter(|stream| {
            stream.is_prepared() && stream.is_seekable() && stream.error().is_none()
        }) {
            stream.seek(position.min(stream.duration().saturating_sub(1).max(0)));
        }
    }

    pub(crate) fn set_volume(&self, volume: f64) {
        if self.closed.get() || !volume.is_finite() {
            return;
        }
        let volume = volume.clamp(0.0, 1.0);
        if self.volume_value.replace(volume) == volume {
            return;
        }
        let stream = self.stream.borrow().clone();
        if let Some(stream) = stream {
            stream.set_volume(volume);
        }
        self.changed_session();
        self.notify();
    }

    pub(crate) fn set_repeat(&self, repeat: Repeat) {
        if self.closed.get() || self.repeat() == repeat {
            return;
        }
        self.queue.borrow_mut().repeat = repeat;
        self.queue_changed();
    }

    pub(crate) fn set_shuffle(&self, shuffle: bool) {
        if self.closed.get() || self.shuffle() == shuffle {
            return;
        }
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
        self.queue.borrow_mut().set_shuffle(shuffle, seed);
        self.queue_changed();
    }

    pub(super) fn replace(&self, tracks: Vec<Track>, position: usize) {
        if self.closed.get() || tracks.is_empty() || position >= tracks.len() {
            return;
        }
        self.identities.borrow_mut().replace(tracks.len());
        self.queue.borrow_mut().replace(tracks, position);
        self.resume_position.set(0);
        if let Some(track) = self.current_track() {
            self.start(track);
        }
    }

    pub(super) fn start(&self, track: Track) {
        self.start_with(track, true);
    }

    pub(super) fn start_with(&self, track: Track, autoplay: bool) {
        if self.closed.get() {
            return;
        }
        let resume_position = self.resume_position.replace(0);
        let was_visible = self.transport_visible.get();
        self.release_stream();
        self.resume_position.set(resume_position);
        self.play_intent.set(autoplay);
        self.transport_visible.set(was_visible);
        self.notice.replace(format!(
            "{} · {}",
            if autoplay { "Now playing" } else { "Paused" },
            track.title()
        ));
        let generation = self.generation.get();
        let stream = PlaybackStream::new_audio(&track.path);
        stream.set_volume(self.volume());
        let weak = self.weak.clone();
        stream.connect_prepared_notify(move |stream| {
            if let Some(controller) = weak.upgrade()
                && controller.generation.get() == generation
                && !controller.closed.get()
                && stream.is_prepared()
            {
                stream.set_volume(controller.volume());
                if resume_position > 0 && stream.is_seekable() {
                    controller.seek(
                        resume_position
                            .min(stream.duration().saturating_sub(1_000_000))
                            .max(0),
                    );
                }
                controller.resume_position.set(0);
                if controller.play_intent.get() {
                    stream.play();
                }
                controller.notify();
            }
        });
        let weak = self.weak.clone();
        stream.connect_playing_notify(move |stream| {
            if let Some(controller) = weak.upgrade()
                && controller.generation.get() == generation
                && stream.is_playing()
            {
                controller.transport_visible.set(true);
                controller.notify();
            }
        });
        let weak = self.weak.clone();
        stream.connect_ended_notify(move |stream| {
            if !stream.is_ended() {
                return;
            }
            let weak = weak.clone();
            glib::idle_add_local_once(move || {
                if let Some(controller) = weak.upgrade()
                    && controller.generation.get() == generation
                    && !controller.closed.get()
                {
                    let next = controller.queue.borrow_mut().advance(true);
                    controller.start_or_finish(next);
                }
            });
        });
        let weak = self.weak.clone();
        stream.connect_error_notify(move |stream| {
            if let Some(controller) = weak.upgrade()
                && controller.generation.get() == generation
                && let Some(error) = stream.error()
            {
                tracing::warn!(%error, "Music playback failed");
                controller
                    .notice
                    .replace(format!("Playback unavailable: {error}"));
                controller.notify();
            }
        });
        let seeking = Rc::new(Cell::new(false));
        let weak = self.weak.clone();
        stream.connect_seeking_notify(move |stream| {
            if let Some(controller) = weak.upgrade()
                && controller.generation.get() == generation
            {
                let was_seeking = seeking.replace(stream.is_seeking());
                if was_seeking && !stream.is_seeking() {
                    controller.discontinuity();
                    controller.changed_session();
                    controller.notify();
                }
            }
        });
        self.stream.replace(Some(stream));
        self.discontinuity();
        self.queue_changed();
        self.waveform_notice
            .replace("Preparing full-track waveform in the background…".into());
        let cancellation = Cancellation::default();
        self.waveform_cancel.replace(cancellation.clone());
        let weak = self.weak.clone();
        glib::MainContext::default().spawn_local(async move {
            let result = gtk::gio::spawn_blocking(move || {
                crate::services::waveform::load(&track.path, &cancellation)
            })
            .await;
            if let Some(controller) = weak.upgrade()
                && controller.generation.get() == generation
                && !controller.closed.get()
            {
                let notice = match result {
                    Ok(Ok(peaks)) => {
                        controller.peaks.replace(peaks);
                        "Now playing waveform · click to seek"
                    }
                    Ok(Err(_)) => "Waveform unavailable; full-track playback still works",
                    Err(_) => "Waveform analysis failed; playback still works",
                };
                controller.waveform_notice.replace(notice.into());
                controller.notify();
            }
        });
    }

    pub(crate) fn play_folder(&self, selected: PathBuf) {
        if self.closed.get() {
            return;
        }
        self.session_revision
            .set(self.session_revision.get().wrapping_add(1));
        self.stop();
        let generation = self.generation.get();
        let cancellation = Cancellation::default();
        self.folder_cancel.replace(cancellation.clone());
        self.notice.replace("Preparing folder queue…".into());
        self.notify();
        let weak = self.weak.clone();
        glib::MainContext::default().spawn_local(async move {
            let result =
                gtk::gio::spawn_blocking(move || read_folder_queue(&selected, &cancellation)).await;
            if let Some(controller) = weak.upgrade()
                && controller.generation.get() == generation
                && !controller.closed.get()
            {
                match result {
                    Ok(Ok(tracks)) if !tracks.is_empty() => controller.replace(tracks, 0),
                    Ok(Err(error)) => {
                        controller
                            .notice
                            .replace(format!("Unable to queue this folder: {error}"));
                    }
                    _ => {
                        controller
                            .notice
                            .replace("The selected song is no longer available".into());
                    }
                }
                controller.notify();
            }
        });
    }

    pub(super) fn remove_queue(&self, index: usize) {
        if self.closed.get() {
            return;
        }
        let (was_current, next, removed) = {
            let mut queue = self.queue.borrow_mut();
            let was_current = queue.current == Some(index);
            let removed = queue.remove(index);
            (was_current, queue.playing().cloned(), removed)
        };
        if !removed {
            return;
        }
        self.identities.borrow_mut().remove(index);
        if was_current {
            self.navigate(next);
        }
        self.queue_changed();
    }

    pub(super) fn move_queue(&self, index: usize, destination: usize) -> bool {
        if self.closed.get() {
            return false;
        }
        let moved = self.queue.borrow_mut().move_before(index, destination);
        if moved {
            self.queue_changed();
        }
        moved
    }

    pub(super) fn clear_queue(&self) {
        if self.closed.get() {
            return;
        }
        self.release_stream();
        self.queue.borrow_mut().replace(Vec::new(), 0);
        self.identities.borrow_mut().replace(0);
        self.resume_position.set(0);
        self.discontinuity();
        self.notice.replace("Queue cleared".into());
        self.queue_changed();
    }

    pub(crate) fn shutdown(&self) {
        if self.closed.replace(true) {
            return;
        }
        self.save_session();
        self.release_stream();
        self.notify();
        if let Some(mut store) = self.session_store.borrow_mut().take() {
            store.finish();
        }
    }
}

impl Drop for Controller {
    fn drop(&mut self) {
        self.shutdown();
        let _cleaned = CONTROLLERS.try_with(|registry| {
            if let Ok(mut registry) = registry.try_borrow_mut() {
                registry.retain(|(app, controller)| {
                    app.upgrade().is_some()
                        && controller
                            .upgrade()
                            .is_some_and(|controller| !controller.closed.get())
                });
            }
        });
    }
}

impl crate::mpris::Backend for Rc<Controller> {
    fn snapshot(&self) -> crate::mpris::Snapshot {
        let capabilities = self.capabilities();
        crate::mpris::Snapshot {
            track: self.current_track(),
            track_id: self.track_id().map_or_else(
                || "/org/mpris/MediaPlayer2/TrackList/NoTrack".into(),
                |(version, occurrence)| {
                    format!("/org/mpris/MediaPlayer2/track/v{version}_{occurrence}")
                },
            ),
            status: match self.status() {
                PlaybackStatus::Stopped => "Stopped",
                PlaybackStatus::Paused => "Paused",
                PlaybackStatus::Playing => "Playing",
            },
            position: self.position(),
            duration: self.duration(),
            volume: self.volume(),
            repeat: self.repeat(),
            shuffle: self.shuffle(),
            can_next: capabilities.can_next,
            can_previous: capabilities.can_previous,
            can_play: capabilities.can_play,
            can_pause: capabilities.can_pause,
            can_seek: capabilities.can_seek,
            seek_revision: self.seek_revision(),
            art_url: None,
        }
    }

    fn command(&self, command: crate::mpris::Command) -> Result<(), String> {
        use crate::mpris::Command;
        if self.closed.get() {
            return Err("Player has shut down".into());
        }
        match command {
            Command::Play => self.play(),
            Command::Pause => self.pause(),
            Command::PlayPause => self.toggle(),
            Command::Stop => self.stop(),
            Command::Next if self.capabilities().can_next => self.next(),
            Command::Previous if self.capabilities().can_previous => self.previous(),
            Command::Next | Command::Previous => {}
            Command::Seek(offset) => {
                if self.capabilities().can_seek {
                    let position = self.position().saturating_add(offset).max(0);
                    if position > self.duration() {
                        self.next();
                    } else {
                        self.seek(position);
                    }
                }
            }
            Command::SetPosition(track_id, position) => {
                if position >= 0
                    && position <= self.duration()
                    && track_id == self.snapshot().track_id
                {
                    self.seek(position);
                }
            }
            Command::SetVolume(volume) => {
                if !volume.is_finite() || volume < 0.0 {
                    return Err("Invalid player volume".into());
                }
                self.set_volume(volume);
            }
            Command::SetRepeat(repeat) => self.set_repeat(repeat),
            Command::SetShuffle(shuffle) => self.set_shuffle(shuffle),
            Command::OpenUri(path) => {
                if !path.is_absolute()
                    || crate::services::classify_by_name(path.as_os_str())
                        != crate::services::FormatFamily::Audio
                {
                    return Err("Expected an absolute local audio path".into());
                }
                self.play_folder(path);
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
