use std::{
    fs::File,
    io::Read,
    path::Path,
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

#[cfg(test)]
#[path = "waveform/tests.rs"]
mod tests;

const BUCKETS: usize = 1024;
const HISTOGRAM_BINS: usize = 8192;
const SAMPLE_RATE: u64 = 16_000;
const MAX_SAMPLES: u64 = SAMPLE_RATE * 60 * 60 * 2;
const DECODE_DEADLINE: Duration = Duration::from_secs(10);

pub(crate) fn analyze(input: &Path) -> Result<Vec<f32>, String> {
    let file = File::open(input).map_err(|error| error.to_string())?;
    if !file
        .metadata()
        .map_err(|error| error.to_string())?
        .is_file()
    {
        return Err("Waveform input is not a regular file".to_owned());
    }
    let mut child = Command::new("/usr/bin/prlimit")
        .args([
            "--as=536870912",
            "--cpu=8",
            "--fsize=0",
            "--nofile=64",
            "--",
            "/usr/bin/ffmpeg",
            "-nostdin",
            "-v",
            "error",
            "-threads",
            "1",
            "-filter_threads",
            "1",
            "-probesize",
            "1048576",
            "-analyzeduration",
            "1000000",
            "-protocol_whitelist",
            "file,pipe",
            "-i",
            "/dev/stdin",
            "-map",
            "0:a:0",
            "-vn",
            "-sn",
            "-dn",
            "-ac",
            "1",
            "-ar",
            "16000",
            "-threads",
            "1",
            "-c:a",
            "pcm_f32le",
            "-f",
            "f32le",
            "pipe:1",
        ])
        .stdin(file)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("Unable to start bounded waveform decoder: {error}"))?;
    let stdout = child.stdout.take().expect("piped decoder stdout");
    let (sender, receiver) = mpsc::sync_channel(1);
    let reader = thread::spawn(move || {
        let result = read_pcm(stdout, MAX_SAMPLES);
        let _sent = sender.send(result);
    });
    let started = Instant::now();
    let result = receiver.recv_timeout(DECODE_DEADLINE);
    let peaks = match result {
        Ok(Ok(peaks)) => peaks,
        error => {
            let _killed = child.kill();
            let _waited = child.wait();
            let _joined = reader.join();
            return Err(match error {
                Ok(Err(message)) => message,
                _ => "Unsupported track: waveform analysis exceeded its time budget".to_owned(),
            });
        }
    };
    let _joined = reader.join();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(peaks),
            Ok(Some(status)) => {
                return Err(format!(
                    "Unsupported track: bounded waveform decoder failed ({status})"
                ));
            }
            Ok(None) if started.elapsed() < DECODE_DEADLINE => {
                thread::sleep(Duration::from_millis(10));
            }
            result => {
                let _killed = child.kill();
                let _waited = child.wait();
                return Err(match result {
                    Err(error) => format!("Unable to monitor waveform decoder: {error}"),
                    _ => "Unsupported track: waveform analysis exceeded its time budget".to_owned(),
                });
            }
        }
    }
}

fn read_pcm(mut reader: impl Read, limit: u64) -> Result<Vec<f32>, String> {
    let mut histogram = Histogram::new();
    let mut bytes = [0_u8; 32 * 1024];
    let mut carried = 0;
    loop {
        let count = match reader.read(&mut bytes[carried..]) {
            Ok(count) => count,
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(format!("Unable to read waveform PCM: {error}")),
        };
        if count == 0 {
            if carried != 0 {
                return Err("Waveform decoder returned incomplete PCM".to_owned());
            }
            return histogram.finish();
        }
        let available = carried + count;
        let aligned = available / 4 * 4;
        for sample in bytes[..aligned].chunks_exact(4) {
            if histogram.samples >= limit {
                return Err("Unsupported long track: waveform sample budget exceeded".to_owned());
            }
            histogram.push(f32::from_le_bytes(
                sample.try_into().expect("four PCM bytes"),
            ))?;
        }
        bytes.copy_within(aligned..available, 0);
        carried = available - aligned;
    }
}

struct Histogram {
    peaks: Vec<f32>,
    width: u64,
    samples: u64,
}

impl Histogram {
    fn new() -> Self {
        Self {
            peaks: vec![0.0; HISTOGRAM_BINS],
            width: 1,
            samples: 0,
        }
    }

    fn push(&mut self, sample: f32) -> Result<(), String> {
        if !sample.is_finite() {
            return Err("Waveform decoder returned non-finite PCM".to_owned());
        }
        if self.samples == self.width * HISTOGRAM_BINS as u64 {
            for index in 0..HISTOGRAM_BINS / 2 {
                self.peaks[index] = self.peaks[index * 2].max(self.peaks[index * 2 + 1]);
            }
            self.peaks[HISTOGRAM_BINS / 2..].fill(0.0);
            self.width *= 2;
        }
        let index = (self.samples / self.width) as usize;
        self.peaks[index] = self.peaks[index].max(sample.abs());
        self.samples += 1;
        Ok(())
    }

    fn finish(self) -> Result<Vec<f32>, String> {
        if self.samples == 0 {
            return Err("Waveform decoder returned no audio samples".to_owned());
        }
        let mut result = vec![0.0_f32; BUCKETS];
        let used = self.samples.div_ceil(self.width) as usize;
        for index in 0..used {
            let start = index as u64 * self.width;
            let end = (start + self.width).min(self.samples);
            let first = (start * BUCKETS as u64 / self.samples) as usize;
            let last = (end * BUCKETS as u64).div_ceil(self.samples) as usize;
            for peak in &mut result[first..last.min(BUCKETS)] {
                *peak = peak.max(self.peaks[index]);
            }
        }
        let maximum = result.iter().copied().fold(0.0_f32, f32::max);
        if maximum > 0.0 {
            for peak in &mut result {
                *peak = (*peak / maximum).clamp(0.0, 1.0);
            }
        }
        Ok(result)
    }
}
