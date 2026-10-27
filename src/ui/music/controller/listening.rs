use super::*;
use std::time::Instant;

#[derive(Default)]
pub(super) struct Listening {
    path: Option<PathBuf>,
    elapsed: i64,
    previous: Option<(Instant, i64)>,
    counted: bool,
}

impl Listening {
    pub fn reset(&mut self, path: Option<PathBuf>) {
        self.path = path;
        self.elapsed = 0;
        self.previous = None;
        self.counted = false;
    }

    pub fn discontinue(&mut self) {
        self.previous = None;
    }

    fn sample(
        &mut self,
        now: Instant,
        position: i64,
        duration: i64,
        eligible: bool,
    ) -> Option<(PathBuf, i64, i64)> {
        if !eligible {
            self.discontinue();
            return None;
        }
        if let Some((previous, timestamp)) = self.previous.replace((now, position)) {
            let wall = now
                .saturating_duration_since(previous)
                .as_micros()
                .min(250_000) as i64;
            let progress = position.saturating_sub(timestamp);
            if (0..=wall.saturating_add(250_000)).contains(&progress) {
                self.elapsed = self.elapsed.saturating_add(progress.min(wall));
            }
        }
        let threshold = if duration > 0 {
            (duration / 2).clamp(1, 30_000_000)
        } else {
            30_000_000
        };
        if !self.counted && self.elapsed >= threshold {
            return Some((self.path.clone()?, self.elapsed, duration));
        }
        None
    }
}

impl Controller {
    pub(super) fn sample_listening(&self) {
        let stream = self.stream.borrow().clone();
        let Some(stream) = stream else {
            return;
        };
        let eligible = stream.is_prepared()
            && stream.is_playing()
            && !stream.is_seeking()
            && !stream.is_ended()
            && stream.error().is_none();
        let record = self.listening.borrow_mut().sample(
            Instant::now(),
            stream.timestamp(),
            stream.duration(),
            eligible,
        );
        if let Some((path, elapsed, duration)) = record {
            match self
                .collection_store
                .borrow()
                .record_listen(path, elapsed, duration)
            {
                Ok(true) => self.listening.borrow_mut().counted = true,
                Ok(false) => {}
                Err(error) => tracing::warn!(%error, "Listening history could not be queued"),
            }
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod desktop_tests;
