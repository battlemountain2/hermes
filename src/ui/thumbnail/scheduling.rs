use super::*;
use std::{cell::Cell, time::Duration};

#[derive(Clone, PartialEq)]
enum Work {
    Resolve(PathBuf, ThumbnailKind, i32),
    Render(ThumbnailSource, ThumbnailKind),
}

impl Work {
    fn render(&self) -> bool {
        matches!(self, Self::Render(..))
    }

    fn restart(&self) -> Self {
        match self {
            Self::Resolve(..) => self.clone(),
            Self::Render(source, kind) => {
                Self::Resolve(source.key.path.clone(), *kind, source.key.thumbnail_size)
            }
        }
    }
}

struct Target {
    image: glib::WeakRef<gtk::Image>,
    image_id: usize,
    request: u64,
    cancellation: Cancellation,
}

impl Target {
    fn alive(&self) -> bool {
        self.image.upgrade().is_some()
            && request_is_current(self.image_id, self.request, &self.cancellation)
    }

    fn priority(&self) -> Option<u8> {
        visible_priority(&self.image.upgrade()?)
    }
}

struct Waiting<Work, Target> {
    work: Work,
    targets: Vec<Target>,
}

struct Flight<Work, Target> {
    id: u64,
    job: Waiting<Work, Target>,
    cancellation: Cancellation,
}

struct Queue<Work, Target> {
    waiting: Vec<Waiting<Work, Target>>,
    running: Vec<Flight<Work, Target>>,
    sequence: u64,
}

impl<Work: Clone + PartialEq, Target> Default for Queue<Work, Target> {
    fn default() -> Self {
        Self {
            waiting: Vec::new(),
            running: Vec::new(),
            sequence: 0,
        }
    }
}

impl<Work: Clone + PartialEq, Target> Queue<Work, Target> {
    fn enqueue(&mut self, work: Work, target: Target, share_running: bool) {
        if share_running
            && let Some(flight) = self
                .running
                .iter_mut()
                .find(|flight| flight.job.work == work && !flight.cancellation.is_cancelled())
        {
            flight.job.targets.push(target);
            return;
        }
        if let Some(job) = self.waiting.iter_mut().find(|job| job.work == work) {
            job.targets.push(target);
        } else {
            self.waiting.push(Waiting {
                work,
                targets: vec![target],
            });
        }
    }

    fn maintain(
        &mut self,
        alive: impl Fn(&Target) -> bool,
        priority: impl Fn(&Target) -> Option<u8>,
    ) {
        for job in &mut self.waiting {
            job.targets.retain(&alive);
        }
        self.waiting.retain(|job| !job.targets.is_empty());
        for flight in &mut self.running {
            flight.job.targets.retain(&alive);
            if !flight
                .job
                .targets
                .iter()
                .any(|target| priority(target).is_some())
            {
                flight.cancellation.cancel();
            }
        }
    }

    fn start(
        &mut self,
        limit: usize,
        priority: impl Fn(&Target) -> Option<u8>,
        prepared: impl Fn(&Work) -> bool,
    ) -> Option<(u64, Work, Cancellation)> {
        if self.running.len() >= limit {
            return None;
        }
        let index = self
            .waiting
            .iter()
            .enumerate()
            .filter_map(|(index, job)| {
                let priority = job.targets.iter().filter_map(&priority).min()?;
                Some((index, (priority, !prepared(&job.work))))
            })
            .min_by_key(|(_, rank)| *rank)?
            .0;
        let job = self.waiting.remove(index);
        self.sequence += 1;
        let id = self.sequence;
        let work = job.work.clone();
        let cancellation = Cancellation::default();
        self.running.push(Flight {
            id,
            job,
            cancellation: cancellation.clone(),
        });
        Some((id, work, cancellation))
    }

    fn finish(&mut self, id: u64) -> Option<Flight<Work, Target>> {
        let index = self.running.iter().position(|flight| flight.id == id)?;
        Some(self.running.remove(index))
    }
}

thread_local! {
    static QUEUE: RefCell<Queue<Work, Target>> = RefCell::new(Queue::default());
    static TICK_PENDING: Cell<bool> = const { Cell::new(false) };
    static TIMER: RefCell<Option<glib::SourceId>> = const { RefCell::new(None) };
}

pub(super) fn enqueue(
    image: &gtk::Image,
    image_id: usize,
    request: u64,
    cancellation: Cancellation,
    path: PathBuf,
    kind: ThumbnailKind,
    size: i32,
) {
    QUEUE.with(|queue| {
        queue.borrow_mut().enqueue(
            Work::Resolve(path, kind, size),
            Target {
                image: image.downgrade(),
                image_id,
                request,
                cancellation,
            },
            false,
        )
    });
    wake();
}

pub(super) fn wake() {
    TIMER.with(|timer| {
        if let Some(source) = timer.borrow_mut().take() {
            source.remove();
        }
    });
    if TICK_PENDING.with(|pending| !pending.replace(true)) {
        glib::idle_add_once(tick);
    }
}

fn visible_priority(image: &gtk::Image) -> Option<u8> {
    if !image.is_mapped() || image.width() <= 0 || image.height() <= 0 {
        return None;
    }
    let mut ancestor = image.parent();
    let mut priority = 0;
    while let Some(widget) = ancestor {
        ancestor = widget.parent();
        if widget.is::<gtk::Popover>() {
            break;
        }
        if let Some(scroll) = widget.downcast_ref::<gtk::ScrolledWindow>() {
            let bounds = image.compute_bounds(scroll)?;
            let width = scroll.width() as f32;
            let height = scroll.height() as f32;
            if bounds.x() + bounds.width() <= -width * 0.5
                || bounds.x() >= width * 1.5
                || bounds.y() + bounds.height() <= -height * 0.5
                || bounds.y() >= height * 1.5
            {
                return None;
            }
            if bounds.x() + bounds.width() <= 0.0
                || bounds.x() >= width
                || bounds.y() + bounds.height() <= 0.0
                || bounds.y() >= height
            {
                priority = 1;
            }
        }
    }
    Some(priority)
}

fn tick() {
    TICK_PENDING.with(|pending| pending.set(false));
    let (starts, interested) = QUEUE.with(|queue| {
        let mut queue = queue.borrow_mut();
        queue.maintain(Target::alive, Target::priority);
        let mut starts = Vec::new();
        while let Some(start) = queue.start(
            crate::sandbox::browser::default_worker_limit(),
            Target::priority,
            Work::render,
        ) {
            starts.push(start);
        }
        let interested = !queue.running.is_empty()
            || queue.waiting.iter().any(|job| {
                job.targets.iter().any(|target| {
                    target
                        .image
                        .upgrade()
                        .is_some_and(|image| image.is_mapped())
                })
            });
        (starts, interested)
    });
    for (id, work, cancellation) in starts {
        launch(id, work, cancellation);
    }
    if interested && TICK_PENDING.with(|pending| !pending.get()) {
        TIMER.with(|timer| {
            if timer.borrow().is_none() {
                timer.replace(Some(glib::timeout_add_once(
                    Duration::from_millis(32),
                    || {
                        TIMER.with(|timer| timer.borrow_mut().take());
                        tick();
                    },
                )));
            }
        });
    }
}

enum Output {
    Resolved(ThumbnailSource, ThumbnailKind),
    Frame(ThumbnailSource, glib::Bytes, bool),
}

fn launch(id: u64, work: Work, cancellation: Cancellation) {
    #[cfg(test)]
    tests::started(work.render());
    let cached = match &work {
        Work::Resolve(path, _, size) => THUMBNAIL_CACHE
            .with(|cache| cache.borrow().candidate(path, *size).map(|key| (key, None))),
        Work::Render(source, _) => THUMBNAIL_CACHE.with(|cache| {
            cache
                .borrow_mut()
                .get(&source.key)
                .map(|bytes| (source.key.clone(), Some(bytes)))
        }),
    };
    glib::MainContext::default().spawn_local(async move {
        let result = gio::spawn_blocking(move || execute(work, cached, &cancellation)).await;
        let flight = QUEUE.with(|queue| queue.borrow_mut().finish(id));
        let Some(flight) = flight else {
            return;
        };
        if flight.cancellation.is_cancelled() {
            QUEUE.with(|queue| {
                let mut queue = queue.borrow_mut();
                for target in flight.job.targets.into_iter().filter(Target::alive) {
                    queue.enqueue(flight.job.work.restart(), target, false);
                }
            });
        } else if let Ok(Ok(output)) = result {
            match output {
                Output::Resolved(source, kind) => QUEUE.with(|queue| {
                    let mut queue = queue.borrow_mut();
                    for target in flight.job.targets.into_iter().filter(Target::alive) {
                        queue.enqueue(Work::Render(source.clone(), kind), target, true);
                    }
                }),
                Output::Frame(source, bytes, persist) => {
                    #[cfg(test)]
                    tests::rendered(persist);
                    let mut persisted = !persist;
                    for target in flight.job.targets.into_iter().filter(Target::alive) {
                        complete_thumbnail(
                            target.image_id,
                            target.request,
                            &target.cancellation,
                            source.key.clone(),
                            bytes.clone(),
                            |bytes| {
                                target.image.upgrade().is_some_and(|image| {
                                    let applied =
                                        apply_thumbnail(&image, bytes, source.key.thumbnail_size);
                                    if applied
                                        && !persisted
                                        && let Some(texture) =
                                            image.paintable().and_downcast::<gdk::Texture>()
                                    {
                                        disk::persist(source.key.clone(), &texture);
                                        persisted = true;
                                    }
                                    applied
                                })
                            },
                        );
                    }
                }
            }
        }
        wake();
    });
}

fn execute(
    work: Work,
    cached: Option<(ThumbnailKey, Option<glib::Bytes>)>,
    cancellation: &Cancellation,
) -> Result<Output, String> {
    if cancellation.is_cancelled() {
        return Err("Thumbnail request was cancelled".into());
    }
    match work {
        Work::Resolve(path, kind, size) => {
            let candidate = cached.map(|(key, _)| key).or_else(|| {
                (kind == ThumbnailKind::AudioAlbum)
                    .then(|| disk::audio_candidate(&path, size))
                    .flatten()
            });
            let source = thumbnail_source(&path, kind, size, candidate.as_ref(), folder_album_art)?;
            if !source.unchanged() || cancellation.is_cancelled() {
                return Err("Thumbnail source changed or request was cancelled".into());
            }
            Ok(Output::Resolved(source, kind))
        }
        Work::Render(source, kind) => {
            if !source.unchanged() {
                return Err("Thumbnail source changed".into());
            }
            let (source, bytes, persist) = if let Some(bytes) = cached.and_then(|(_, bytes)| bytes)
            {
                (source, bytes, false)
            } else if let Some(bytes) = disk::load(&source.key) {
                (source, glib::Bytes::from_owned(bytes), false)
            } else {
                let (source, bytes) = render_thumbnail_with_cover(
                    source,
                    kind,
                    cancellation,
                    |path, operation, size, cancellation| {
                        crate::sandbox::parse(path, operation, size, cancellation)
                            .map(|output| output.data)
                    },
                )?;
                (source, glib::Bytes::from_owned(bytes), true)
            };
            if !source.unchanged() || cancellation.is_cancelled() {
                return Err("Thumbnail source changed or request was cancelled".into());
            }
            Ok(Output::Frame(source, bytes, persist))
        }
    }
}

#[cfg(test)]
mod tests;
