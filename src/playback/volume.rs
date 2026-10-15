use super::*;

pub(super) fn preserve_pcm_silence(element: &gst::Element) -> Result<(), String> {
    element
        .static_pad("src")
        .ok_or("Audio volume output missing")?
        .add_probe(gst::PadProbeType::BUFFER, |_, info| {
            if let Some(buffer) = info.buffer_mut()
                && buffer.flags().contains(gst::BufferFlags::GAP)
                && buffer.size() > 0
            {
                buffer.make_mut().unset_flags(gst::BufferFlags::GAP);
            }
            gst::PadProbeReturn::Ok
        })
        .ok_or("Unable to preserve silent audio output")?;
    Ok(())
}

pub(super) struct Control {
    latest: Arc<AtomicU64>,
    wake: mpsc::SyncSender<()>,
}

impl Control {
    fn new() -> (Self, mpsc::Receiver<()>) {
        let (wake, receiver) = mpsc::sync_channel(1);
        (
            Self {
                latest: Arc::new(AtomicU64::new(encode(true, 0.0))),
                wake,
            },
            receiver,
        )
    }

    pub fn start(element: gst::Element, stopped: Arc<AtomicBool>) -> Self {
        let (control, receiver) = Self::new();
        let latest = control.latest.clone();
        std::thread::spawn(move || {
            consume(receiver, &latest, &stopped, |muted, volume| {
                if muted {
                    element.set_property("mute", true);
                }
                element.set_property("volume", volume);
                if !muted {
                    element.set_property("mute", false);
                }
            });
        });
        control
    }

    pub fn set(&self, muted: bool, volume: f64) {
        self.latest.store(encode(muted, volume), Ordering::Release);
        self.wake();
    }

    pub fn wake(&self) {
        let _sent = self.wake.try_send(());
    }
}

fn encode(muted: bool, volume: f64) -> u64 {
    let volume = if volume.is_finite() {
        volume.clamp(0.0, 1.0).max(0.0)
    } else {
        0.0
    };
    volume.to_bits() | (u64::from(muted) << 63)
}

fn consume(
    receiver: mpsc::Receiver<()>,
    latest: &AtomicU64,
    stopped: &AtomicBool,
    mut apply: impl FnMut(bool, f64),
) {
    while receiver.recv().is_ok() {
        if stopped.load(Ordering::Acquire) {
            break;
        }
        let settings = latest.load(Ordering::Acquire);
        apply(settings >> 63 != 0, f64::from_bits(settings & !(1 << 63)));
    }
}

#[cfg(test)]
mod tests;
