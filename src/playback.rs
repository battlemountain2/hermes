use std::{
    io::{self, Write},
    path::Path,
    process::{Child, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

use gstreamer::{self as gst, prelude::*};
use gstreamer_app as app;

pub mod spectrum;
mod volume;
mod wire;
pub(crate) mod worker;

pub use spectrum::SPECTRUM_BANDS;

use wire::{Command, Event};

pub(crate) fn warmup() -> Option<std::thread::JoinHandle<Result<(), String>>> {
    static STARTED: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    STARTED.set(()).ok()?;
    std::thread::Builder::new()
        .name("hermes-media-warmup".into())
        .spawn(|| {
            let result = gst::init().map_err(|error| error.to_string());
            if let Err(error) = &result {
                tracing::warn!(%error, "media initialization failed");
            }
            result
        })
        .map_err(|error| tracing::warn!(%error, "unable to start media initialization"))
        .ok()
}

#[derive(Default)]
pub(crate) struct Updates {
    pub prepared: Option<(i64, bool, bool, bool)>,
    pub frame: Option<(i64, u32, u32, Vec<u8>)>,
    pub position: Option<i64>,
    pub seeked: Option<bool>,
    pub ended: bool,
    pub error: Option<String>,
}

pub(crate) struct Session {
    updates: Arc<Mutex<Updates>>,
    commands: mpsc::SyncSender<Command>,
    stopped: Arc<AtomicBool>,
    epoch: Arc<AtomicU64>,
    playing: Arc<AtomicBool>,
    waiting: Arc<AtomicBool>,
    waiting_since: Arc<AtomicU64>,
    heartbeat: Arc<AtomicU64>,
    started: Instant,
    volume: volume::Control,
    spectrum: Arc<Mutex<spectrum::SpectrumAnalyzer>>,
}

impl Session {
    pub fn start(path: &Path) -> Result<Self, String> {
        let path = path.canonicalize().map_err(|error| error.to_string())?;
        if !path.is_file() {
            return Err("Playback input is not a regular file".into());
        }
        gst::init().map_err(|error| error.to_string())?;
        let audio = gst::parse::launch(
            "appsrc name=pcm is-live=true format=time do-timestamp=true block=false max-bytes=96000 max-buffers=16 leaky-type=downstream caps=audio/x-raw,format=F32LE,rate=48000,channels=2,layout=interleaved ! volume name=level ! autoaudiosink sync=false")
            .map_err(|error| error.to_string())?.downcast::<gst::Pipeline>().map_err(|_| "Audio pipeline missing")?;
        let pcm = audio
            .by_name("pcm")
            .unwrap()
            .downcast::<app::AppSrc>()
            .unwrap();
        let volume = audio.by_name("level").unwrap();
        volume.set_property("mute", true);
        volume.set_property("volume", 0.0_f64);
        volume::preserve_pcm_silence(&volume)?;
        let (commands, receiver) = mpsc::sync_channel(32);
        let updates = Arc::new(Mutex::new(Updates::default()));
        let audio_updates = updates.clone();
        if let Some(bus) = audio.bus() {
            bus.set_sync_handler(move |_, message| {
                if let gst::MessageView::Error(error) = message.view() {
                    audio_updates.lock().unwrap().error =
                        Some(format!("Audio output failed: {}", error.error()));
                }
                gst::BusSyncReply::Drop
            });
        }
        let stopped = Arc::new(AtomicBool::new(false));
        let volume = volume::Control::start(volume, stopped.clone());
        let spectrum = Arc::new(Mutex::new(spectrum::SpectrumAnalyzer::new()));
        let epoch = Arc::new(AtomicU64::new(0));
        let playing = Arc::new(AtomicBool::new(false));
        let waiting = Arc::new(AtomicBool::new(true));
        let waiting_since = Arc::new(AtomicU64::new(0));
        let heartbeat = Arc::new(AtomicU64::new(0));
        let started = Instant::now();
        let mut command = sandbox_command(&path)?;
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let (startup, started_decoder) = mpsc::sync_channel(1);
        let monitor_updates = updates.clone();
        let monitor_heartbeat = heartbeat.clone();
        let monitor_stopped = stopped.clone();
        let monitor_playing = playing.clone();
        let monitor_waiting = waiting.clone();
        let monitor_waiting_since = waiting_since.clone();
        std::thread::spawn(move || {
            let mut child = match command.spawn() {
                Ok(child) => child,
                Err(error) => {
                    let _sent =
                        startup.send(Err(format!("Unable to start isolated playback: {error}")));
                    return;
                }
            };
            let pipes = match (child.stdin.take(), child.stdout.take()) {
                (Some(input), Some(output)) => Ok((input, output)),
                _ => Err("Playback pipes missing".to_owned()),
            };
            let valid = pipes.is_ok();
            if startup.send(pipes).is_err() || !valid {
                let _killed = child.kill();
                let _reaped = child.wait();
                return;
            }
            monitor(
                &mut child,
                &monitor_stopped,
                &monitor_playing,
                &monitor_waiting,
                &monitor_waiting_since,
                &monitor_heartbeat,
                started,
                &monitor_updates,
            );
        });
        let (mut input, mut output) = started_decoder
            .recv()
            .map_err(|_| "Playback startup thread disconnected")??;
        let control_audio = audio.clone();
        let control_updates = updates.clone();
        let control_stopped = stopped.clone();
        std::thread::spawn(move || {
            while let Ok(command) = receiver.recv() {
                if control_stopped.load(Ordering::Acquire) {
                    break;
                }
                if matches!(command, Command::Stop) {
                    break;
                }
                if control_audio.set_state(output_state(&command)).is_err() {
                    control_updates.lock().unwrap().error =
                        Some("Audio output could not change playback state".to_owned());
                    break;
                }
                let result = (|| -> io::Result<()> {
                    serde_json::to_writer(&mut input, &command)?;
                    input.write_all(b"\n")?;
                    input.flush()
                })();
                if let Err(error) = result {
                    control_updates.lock().unwrap().error =
                        Some(format!("Playback control failed: {error}"));
                    break;
                }
            }
            let _state = control_audio.set_state(gst::State::Null);
        });
        let reader_updates = updates.clone();
        let reader_stopped = stopped.clone();
        let reader_epoch = epoch.clone();
        let reader_waiting = waiting.clone();
        let reader_playing = playing.clone();
        let reader_heartbeat = heartbeat.clone();
        let reader_audio = audio.clone();
        let reader_spectrum = spectrum.clone();
        std::thread::spawn(move || {
            while !reader_stopped.load(Ordering::Acquire) {
                let (event, data) = match wire::read_event(&mut output) {
                    Ok(packet) => packet,
                    Err(error) => {
                        if !reader_stopped.load(Ordering::Acquire) {
                            reader_updates.lock().unwrap().error.get_or_insert_with(|| {
                                format!("Playback decoder disconnected: {error}")
                            });
                        }
                        break;
                    }
                };
                reader_heartbeat.store(started.elapsed().as_millis() as u64, Ordering::Release);
                let current = reader_epoch.load(Ordering::Acquire);
                match event {
                    Event::Prepared {
                        duration,
                        audio,
                        video,
                        seekable,
                    } => {
                        reader_waiting.store(false, Ordering::Release);
                        reader_updates.lock().unwrap().prepared =
                            Some((duration, audio, video, seekable));
                    }
                    Event::Frame {
                        epoch,
                        timestamp,
                        width,
                        height,
                    } if epoch == current => {
                        let mut updates = reader_updates.lock().unwrap();
                        if reader_epoch.load(Ordering::Acquire) == epoch {
                            updates.frame = Some((timestamp, width, height, data));
                        }
                    }
                    Event::Audio { epoch }
                        if epoch == current && reader_playing.load(Ordering::Acquire) =>
                    {
                        if !reader_waiting.load(Ordering::Acquire) {
                            reader_spectrum.lock().unwrap().push_pcm(&data);
                            let _pushed = pcm.push_buffer(pcm_buffer(data));
                        }
                    }
                    Event::Position { epoch, timestamp } if epoch == current => {
                        let mut updates = reader_updates.lock().unwrap();
                        if reader_epoch.load(Ordering::Acquire) == epoch {
                            updates.position = Some(timestamp);
                        }
                    }
                    Event::Seeked { epoch, success } if epoch == current => {
                        reader_spectrum.lock().unwrap().reset();
                        reader_waiting.store(false, Ordering::Release);
                        if reader_playing.load(Ordering::Acquire) {
                            let _state = reader_audio.set_state(gst::State::Playing);
                        }
                        reader_updates.lock().unwrap().seeked = Some(success);
                    }
                    Event::Ended { epoch } if epoch == current => {
                        reader_playing.store(false, Ordering::Release);
                        reader_updates.lock().unwrap().ended = true;
                    }
                    Event::Error { message } => {
                        reader_updates.lock().unwrap().error = Some(message);
                    }
                    _ => {}
                }
            }
            let _state = reader_audio.set_state(gst::State::Null);
        });
        Ok(Self {
            updates,
            commands,
            stopped,
            epoch,
            playing,
            waiting,
            waiting_since,
            heartbeat,
            started,
            volume,
            spectrum,
        })
    }

    pub fn take_updates(&self) -> Updates {
        std::mem::take(&mut *self.updates.lock().unwrap())
    }
    pub fn play(&self) -> bool {
        self.heartbeat
            .store(self.started.elapsed().as_millis() as u64, Ordering::Release);
        self.playing.store(true, Ordering::Release);
        self.send(Command::Play)
    }
    pub fn pause(&self) {
        self.playing.store(false, Ordering::Release);
        let _sent = self.send(Command::Pause);
    }
    pub fn seek(&self, timestamp: i64) -> bool {
        self.spectrum.lock().unwrap().reset();
        self.heartbeat
            .store(self.started.elapsed().as_millis() as u64, Ordering::Release);
        let epoch = self.epoch.fetch_add(1, Ordering::AcqRel).wrapping_add(1);
        *self.updates.lock().unwrap() = Updates::default();
        self.waiting_since
            .store(self.started.elapsed().as_millis() as u64, Ordering::Release);
        self.waiting.store(true, Ordering::Release);
        self.send(Command::Seek { epoch, timestamp })
    }
    pub fn spectrum_bands(&self) -> [f32; SPECTRUM_BANDS] {
        if self.stopped.load(Ordering::Acquire) {
            return [0.0; SPECTRUM_BANDS];
        }
        let mut analyzer = self.spectrum.lock().unwrap();
        if self.playing.load(Ordering::Acquire) {
            analyzer.analyze()
        } else {
            analyzer.decay()
        }
    }
    pub fn set_volume(&self, muted: bool, volume: f64) {
        if !self.stopped.load(Ordering::Acquire) {
            self.volume.set(muted, volume);
        }
    }
    fn send(&self, command: Command) -> bool {
        if self.stopped.load(Ordering::Acquire) {
            return false;
        }
        match self.commands.try_send(command) {
            Ok(()) => true,
            Err(_) => {
                self.updates.lock().unwrap().error =
                    Some("Playback command queue unavailable".into());
                false
            }
        }
    }
    pub fn stop(&self) {
        self.stopped.store(true, Ordering::Release);
        self.volume.wake();
        self.playing.store(false, Ordering::Release);
        let _sent = self.commands.try_send(Command::Stop);
    }
}

fn output_state(command: &Command) -> gst::State {
    match command {
        Command::Play => gst::State::Playing,
        Command::Pause => gst::State::Paused,
        Command::Seek { .. } => gst::State::Ready,
        Command::Stop => gst::State::Null,
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop();
    }
}

fn monitor(
    child: &mut Child,
    stopped: &AtomicBool,
    playing: &AtomicBool,
    waiting: &AtomicBool,
    waiting_since: &AtomicU64,
    heartbeat: &AtomicU64,
    started: Instant,
    updates: &Mutex<Updates>,
) {
    loop {
        if stopped.load(Ordering::Acquire) {
            break;
        }
        if let Ok(Some(status)) = child.try_wait() {
            std::thread::sleep(Duration::from_millis(50));
            let mut updates = updates.lock().unwrap();
            if updates
                .error
                .as_ref()
                .is_none_or(|error| error.starts_with("Playback decoder disconnected:"))
            {
                let message = format!("Playback decoder exited unexpectedly: {status}");
                tracing::warn!(%status, "Isolated playback worker exited");
                updates.error = Some(message);
            }
            break;
        }
        let silent = (started.elapsed().as_millis() as u64)
            .saturating_sub(heartbeat.load(Ordering::Acquire));
        let pending = (started.elapsed().as_millis() as u64)
            .saturating_sub(waiting_since.load(Ordering::Acquire));
        if (playing.load(Ordering::Acquire) && silent > 12_000)
            || (waiting.load(Ordering::Acquire) && pending > 12_000)
        {
            updates.lock().unwrap().error = Some("Playback decoder stopped responding".into());
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let _killed = child.kill();
    let _reaped = child.wait();
}

fn sandbox_command(path: &Path) -> Result<std::process::Command, String> {
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    #[cfg(test)]
    let executable = executable
        .parent()
        .and_then(Path::parent)
        .ok_or("Test executable location missing")?
        .join("hermes");
    let mut command = std::process::Command::new("bwrap");
    command.args([
        "--unshare-all",
        "--die-with-parent",
        "--new-session",
        "--clearenv",
        "--setenv",
        "PATH",
        "/usr/bin",
        "--setenv",
        "HOME",
        "/nonexistent",
        "--setenv",
        "XDG_CACHE_HOME",
        "/tmp/cache",
        "--setenv",
        "GST_REGISTRY",
        "/tmp/registry.bin",
        "--proc",
        "/proc",
        "--dev",
        "/dev",
        "--tmpfs",
        "/tmp",
        "--dir",
        "/app",
        "--ro-bind",
        "/usr",
        "/usr",
        "--ro-bind-try",
        "/lib",
        "/lib",
        "--ro-bind-try",
        "/lib64",
        "/lib64",
        "--ro-bind-try",
        "/etc/ld.so.cache",
        "/etc/ld.so.cache",
    ]);
    let registry = std::env::var_os("GST_REGISTRY")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            gtk::glib::user_cache_dir()
                .join("gstreamer-1.0")
                .join(format!("registry.{}.bin", std::env::consts::ARCH))
        });
    if let Ok(registry) = registry.canonicalize()
        && registry.is_file()
    {
        command
            .arg("--ro-bind")
            .arg(registry)
            .arg("/app/registry.bin")
            .args([
                "--setenv",
                "GST_REGISTRY",
                "/app/registry.bin",
                "--setenv",
                "GST_REGISTRY_UPDATE",
                "no",
            ]);
    }
    command
        .arg("--ro-bind")
        .arg(executable)
        .arg("/app/strata")
        .arg("--ro-bind")
        .arg(path)
        .arg("/input")
        .args([
            "--",
            "/usr/bin/prlimit",
            "--as=1342177280",
            "--fsize=33554432",
            "--nofile=128",
            "--",
            "/app/strata",
            "--playback-worker",
        ]);
    Ok(command)
}

fn pcm_buffer(mut data: Vec<u8>) -> gst::Buffer {
    sanitize_pcm(&mut data);
    let frames = data.len() as u64 / 8;
    let mut buffer = gst::Buffer::from_mut_slice(data);
    buffer
        .get_mut()
        .unwrap()
        .set_duration(gst::ClockTime::from_nseconds(
            frames * gst::ClockTime::SECOND.nseconds() / 48_000,
        ));
    buffer
}

fn sanitize_pcm(data: &mut [u8]) {
    for sample in data.chunks_exact_mut(4) {
        let value = f32::from_le_bytes(sample.try_into().unwrap());
        sample.copy_from_slice(
            &if value.is_finite() {
                value.clamp(-1.0, 1.0)
            } else {
                0.0
            }
            .to_le_bytes(),
        );
    }
}

#[cfg(test)]
mod tests;
