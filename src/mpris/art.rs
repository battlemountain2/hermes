use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    fs::{self, DirBuilder, OpenOptions},
    io::Write,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::PathBuf,
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use gtk::{gio, glib, prelude::*};

use crate::{music::Track, sandbox::Cancellation};

pub(super) struct Artwork(Rc<State>);

struct State {
    album: RefCell<Option<PathBuf>>,
    generation: Cell<u64>,
    cancellation: RefCell<Cancellation>,
    current: RefCell<Option<Arc<SavedArt>>>,
    previous: RefCell<VecDeque<Arc<SavedArt>>>,
}

struct SavedArt {
    directory: PathBuf,
    path: PathBuf,
}

impl Drop for SavedArt {
    fn drop(&mut self) {
        let _removed = fs::remove_dir_all(&self.directory);
    }
}

impl Artwork {
    pub fn new() -> Self {
        Self(Rc::new(State {
            album: RefCell::new(None),
            generation: Cell::new(0),
            cancellation: RefCell::new(Cancellation::default()),
            current: RefCell::new(None),
            previous: RefCell::new(VecDeque::new()),
        }))
    }

    pub fn url(&self, track: &Option<Track>) -> Option<String> {
        let track = track.as_ref()?;
        if self.0.album.borrow().as_ref() != Some(&track.album_key()) {
            return None;
        }
        self.0
            .current
            .borrow()
            .as_ref()
            .map(|art| gio::File::for_path(&art.path).uri().to_string())
    }

    pub fn update(&self, track: Option<&Track>) {
        let album = track.map(Track::album_key);
        if *self.0.album.borrow() == album {
            return;
        }
        self.0.cancellation.borrow().cancel();
        self.0
            .generation
            .set(self.0.generation.get().wrapping_add(1));
        self.0.album.replace(album.clone());
        let previous = self.0.current.borrow_mut().take();
        if let Some(previous) = previous {
            let mut retained = self.0.previous.borrow_mut();
            retained.push_back(previous);
            while retained.len() > 2 {
                retained.pop_front();
            }
        }
        let (Some(album), Some(track)) = (album, track) else {
            return;
        };
        let path = track.path.clone();
        let cancellation = Cancellation::default();
        self.0.cancellation.replace(cancellation.clone());
        let generation = self.0.generation.get();
        let weak = Rc::downgrade(&self.0);
        glib::MainContext::default().spawn_local(async move {
            let result = gio::spawn_blocking(move || {
                let (source, operation) = crate::ui::thumbnail::folder_album_art(&album)
                    .unwrap_or((path, crate::sandbox::ParseOperation::ThumbnailVideo));
                let png = crate::sandbox::parse(&source, operation, 128, &cancellation)?.data;
                if cancellation.is_cancelled() {
                    return Err("Artwork cancelled".into());
                }
                save_png(&png)
                    .map(Arc::new)
                    .map_err(|error| error.to_string())
            })
            .await;
            if let Some(state) = weak.upgrade()
                && state.generation.get() == generation
                && let Ok(Ok(art)) = result
            {
                state.current.replace(Some(art));
            }
        });
    }
}

impl Drop for Artwork {
    fn drop(&mut self) {
        self.0.cancellation.borrow().cancel();
    }
}

fn save_png(png: &[u8]) -> std::io::Result<SavedArt> {
    if png.len() > 1024 * 1024 || !png.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Invalid artwork payload",
        ));
    }
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let directory = std::env::temp_dir().join(format!(
        "hermes-mpris-art-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    DirBuilder::new().mode(0o700).create(&directory)?;
    let art = SavedArt {
        path: directory.join("cover.png"),
        directory,
    };
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(&art.path)?;
    file.write_all(png)?;
    Ok(art)
}

#[cfg(test)]
mod tests;
