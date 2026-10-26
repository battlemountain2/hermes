// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    fs, io,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

pub(crate) mod browser;
mod helper_binary;
pub(crate) mod office;

pub(crate) const WALL_TIME_LIMIT: Duration = Duration::from_secs(12);
pub(crate) const MAX_OUTPUT_BYTES: u64 = 32 * 1024 * 1024;
pub(crate) const MAX_TEXT_LAYER_BYTES: u64 = 8 * 1024 * 1024;
pub(crate) const FILE_SIZE_LIMIT_BYTES: u64 = 32 * 1024 * 1024;
pub(crate) const ADDRESS_SPACE_LIMIT_BYTES: u64 = 1_342_177_280;
pub(crate) const CPU_TIME_LIMIT_SECS: u64 = 10;
const OFFICE_ADDRESS_SPACE_LIMIT_BYTES: u64 = 4_294_967_296;
const OFFICE_DATA_LIMIT_BYTES: u64 = 536_870_912;
const OFFICE_CPU_TIME_LIMIT_SECS: u64 = 30;
const OFFICE_WALL_TIME_LIMIT: Duration = Duration::from_secs(35);
static NEXT_DIRECTORY: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ParseOperation {
    ThumbnailImage,
    ThumbnailHeif,
    ThumbnailRaw,
    ThumbnailPdf,
    ThumbnailOffice,
    OfficePoster,
    ThumbnailVideo,
    PreviewImage,
    PreviewGeoTiff,
    PreviewHeif,
    PreviewPdf,
    PreviewAnimation,
    PreviewAudio,
    ExtractPdfText,
    PreviewArchive,
    PreviewOffice,
    ConvertOfficePdf,
    ExtractOfficeText,
    PreviewModel,
    PreviewArchiveCover,
    PreviewSpreadsheet,
    PreviewAudioMetadata,
    ImageMetadata,
    MediaMetadata,
    PreviewAudioArtwork,
    MusicTags,
    MusicWaveform,
    ExtractSpreadsheetText,
}

impl ParseOperation {
    fn uses_office_renderer(self) -> bool {
        matches!(self, Self::ConvertOfficePdf | Self::ThumbnailOffice)
    }

    fn argument(self) -> &'static str {
        match self {
            Self::MusicTags => "music-tags",
            Self::MusicWaveform => "music-waveform",
            Self::ThumbnailImage => "thumbnail-image",
            Self::ThumbnailHeif => "thumbnail-heif",
            Self::ThumbnailRaw => "thumbnail-raw",
            Self::ThumbnailPdf => "thumbnail-pdf",
            Self::ThumbnailOffice => "thumbnail-office",
            Self::OfficePoster => "office-poster",
            Self::ThumbnailVideo => "thumbnail-video",
            Self::PreviewImage => "preview-image",
            Self::PreviewGeoTiff => "preview-geotiff",
            Self::PreviewHeif => "preview-heif",
            Self::PreviewPdf => "preview-pdf",
            Self::PreviewAnimation => "preview-animation",
            Self::PreviewAudio => "preview-audio",
            Self::ExtractPdfText => "extract-pdf-text",
            Self::PreviewArchive => "preview-archive",
            Self::PreviewOffice => "preview-office",
            Self::ConvertOfficePdf => "convert-office-pdf",
            Self::ExtractOfficeText => "extract-office-text",
            Self::PreviewModel => "preview-model",
            Self::PreviewArchiveCover => "preview-archive-cover",
            Self::PreviewSpreadsheet => "preview-spreadsheet",
            Self::PreviewAudioMetadata => "preview-audio-metadata",
            Self::ImageMetadata => "image-metadata",
            Self::MediaMetadata => "media-metadata",
            Self::PreviewAudioArtwork => "preview-audio-artwork",
            Self::ExtractSpreadsheetText => "extract-spreadsheet-text",
        }
    }

    fn output_name(self) -> &'static str {
        if self == Self::ConvertOfficePdf {
            return "result.pdf";
        }
        if matches!(
            self,
            Self::MusicTags | Self::MusicWaveform | Self::PreviewAudioMetadata | Self::ImageMetadata | Self::MediaMetadata
        ) {
            "result.json"
        } else if self == Self::PreviewAnimation {
            "result.webm"
        } else if self == Self::PreviewAudio {
            "result.ogg"
        } else if matches!(
            self,
            Self::ExtractPdfText
                | Self::PreviewArchive
                | Self::PreviewOffice
                | Self::ExtractOfficeText
                | Self::ExtractSpreadsheetText
        ) {
            "result.txt"
        } else {
            "result.png"
        }
    }
}

#[derive(Clone, Default)]
pub(crate) struct Cancellation(Arc<AtomicBool>);

impl Cancellation {
    pub(crate) fn from_shared(cancelled: Arc<AtomicBool>) -> Self {
        Self(cancelled)
    }

    pub(crate) fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

pub(crate) struct ParseOutput {
    pub(crate) data: Vec<u8>,
    pub(crate) page: i32,
    pub(crate) pages: i32,
    pub(crate) metadata: Option<Vec<u8>>,
    pub(crate) text_layer: Option<crate::services::preview::PdfTextLayer>,
}

pub(crate) fn parse(
    input: &Path,
    operation: ParseOperation,
    value: i32,
    cancellation: &Cancellation,
) -> Result<ParseOutput, String> {
    if cancellation.is_cancelled() {
        return Err("Preview cancelled".to_owned());
    }
    let input = input
        .canonicalize()
        .map_err(|error| format!("Unable to open preview input: {error}"))?;
    if !input.is_file() {
        return Err("Preview input is not a regular file".to_owned());
    }

    if let Some(result) = browser::preview(&input, &operation, cancellation) {
        return result.map(|(data, metadata)| ParseOutput {
            data,
            page: 0,
            pages: 0,
            metadata,
            text_layer: None,
        });
    }

    if let Some(result) = browser::thumbnail_parse(&input, operation, cancellation) {
        return result.map(|data| ParseOutput {
            data,
            page: 0,
            pages: 0,
            metadata: None,
            text_layer: None,
        });
    }

    if operation.uses_office_renderer() {
        return office::admit(
            cancellation,
            operation == ParseOperation::ThumbnailOffice,
            || parse_one_shot(&input, operation, value, cancellation),
        );
    }
    parse_one_shot(&input, operation, value, cancellation)
}

fn parse_one_shot(
    input: &Path,
    operation: ParseOperation,
    value: i32,
    cancellation: &Cancellation,
) -> Result<ParseOutput, String> {
    let executable = helper_executable()
        .map_err(|error| format!("Unable to locate the Hermes executable: {error}"))?;
    parse_one_shot_with_executable(input, operation, value, cancellation, &executable)
}

pub(super) fn helper_executable() -> io::Result<PathBuf> {
    let executable = std::env::current_exe()?;
    #[cfg(test)]
    let executable = executable
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| io::Error::other("Test executable location missing"))?
        .join("hermes");
    Ok(executable)
}

pub(crate) fn parse_one_shot_with_executable(
    input: &Path,
    operation: ParseOperation,
    value: i32,
    cancellation: &Cancellation,
    executable: &Path,
) -> Result<ParseOutput, String> {
    let output = PrivateOutput::create().map_err(|error| error.to_string())?;
    let command = sandbox_command(executable, input, output.path(), operation, value)
        .map_err(|error| format!("Unable to prepare the preview helper: {error}"))?;
    run_one_shot(command, &output, operation, cancellation)
}

fn run_one_shot(
    mut command: Command,
    output: &PrivateOutput,
    operation: ParseOperation,
    cancellation: &Cancellation,
) -> Result<ParseOutput, String> {
    let diagnostic_path = output.path().join("renderer.stderr");
    let diagnostic = fs::File::create(&diagnostic_path).map_err(|error| error.to_string())?;
    let mut child = command
        .stdout(Stdio::null())
        .stderr(Stdio::from(diagnostic))
        .spawn()
        .map_err(|error| format!("Unable to start the preview sandbox: {error}"))?;
    let started = Instant::now();

    let status = loop {
        if cancellation.is_cancelled() {
            terminate(&mut child);
            return Err("Preview cancelled".to_owned());
        }
        if started.elapsed()
            >= if operation.uses_office_renderer() {
                OFFICE_WALL_TIME_LIMIT
            } else {
                WALL_TIME_LIMIT
            }
        {
            terminate(&mut child);
            return Err("The preview renderer timed out".to_owned());
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => thread::sleep(Duration::from_millis(20)),
            Err(error) => {
                terminate(&mut child);
                return Err(format!("Unable to monitor the preview renderer: {error}"));
            }
        }
    };
    if !status.success() {
        let mut diagnostic = Vec::new();
        if let Ok(file) = fs::File::open(&diagnostic_path) {
            let _read = file.take(4096).read_to_end(&mut diagnostic);
        }
        let message = renderer_failure_message(status, &diagnostic);
        if operation == ParseOperation::OfficePoster {
            tracing::debug!(%status, "No saved Office poster was available");
        } else {
            tracing::warn!(?operation, %status, "preview renderer failed");
        }
        return Err(message);
    }

    let result_path = output.path().join(operation.output_name());
    let metadata = fs::metadata(&result_path)
        .map_err(|_| "The preview renderer produced no output".to_owned())?;
    let text_output = matches!(
        operation,
        ParseOperation::ExtractPdfText
            | ParseOperation::PreviewArchive
            | ParseOperation::PreviewOffice
            | ParseOperation::ExtractOfficeText
            | ParseOperation::ExtractSpreadsheetText
    );
    let permits_empty_output = text_output || operation == ParseOperation::PreviewSpreadsheet;
    if (!permits_empty_output && metadata.len() == 0) || metadata.len() > MAX_OUTPUT_BYTES {
        return Err("The preview renderer produced an invalid output size".to_owned());
    }
    let data = fs::read(result_path).map_err(|error| error.to_string())?;
    if operation == ParseOperation::ConvertOfficePdf && !data.starts_with(b"%PDF-") {
        return Err("The Office converter produced invalid PDF data".into());
    }
    if !matches!(
        operation,
        ParseOperation::MusicTags
            | ParseOperation::PreviewAudioMetadata
            | ParseOperation::ImageMetadata
            | ParseOperation::MediaMetadata
            | ParseOperation::MusicWaveform
            | ParseOperation::PreviewAnimation
            | ParseOperation::PreviewAudio
            | ParseOperation::ExtractPdfText
            | ParseOperation::PreviewArchive
            | ParseOperation::PreviewOffice
            | ParseOperation::ConvertOfficePdf
            | ParseOperation::ExtractOfficeText
            | ParseOperation::ExtractSpreadsheetText
            | ParseOperation::PreviewSpreadsheet
    ) && !data.starts_with(b"\x89PNG\r\n\x1a\n")
    {
        return Err("The preview renderer produced invalid image data".to_owned());
    }
    if text_output && (data.contains(&0) || std::str::from_utf8(&data).is_err()) {
        return Err("The document text extractor produced invalid text".to_owned());
    }
    let meta_path = output.path().join("result.meta");
    let (page, pages) = read_metadata(&meta_path);
    let metadata = fs::read(&meta_path).ok();
    let text_layer = if operation == ParseOperation::PreviewPdf {
        let text_path = output.path().join("result.text");
        if let Ok(bytes) = fs::read(&text_path) {
            if (bytes.len() as u64) <= MAX_TEXT_LAYER_BYTES {
                serde_json::from_slice(&bytes).ok()
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };
    Ok(ParseOutput {
        data,
        page,
        pages,
        metadata,
        text_layer,
    })
}

fn renderer_failure_message(status: std::process::ExitStatus, diagnostic: &[u8]) -> String {
    let diagnostic = String::from_utf8_lossy(diagnostic);
    let detail: String = diagnostic
        .chars()
        .filter(|character| !character.is_control() || *character == '\n')
        .take(2048)
        .collect();
    let detail = detail.trim();
    if detail.is_empty() {
        format!(
            "The sandboxed preview renderer failed ({status}). It may have exceeded a resource limit."
        )
    } else {
        format!("The sandboxed preview renderer failed ({status}): {detail}")
    }
}

fn sandbox_command(
    executable: &Path,
    input: &Path,
    output: &Path,
    operation: ParseOperation,
    value: i32,
) -> io::Result<Command> {
    let input_path = sandbox_input_path(input);
    let mut command = Command::new("bwrap");
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
        "--proc",
        "/proc",
        "--dev",
        "/dev",
        "--tmpfs",
        "/tmp",
        "--dir",
        "/app",
        "--dir",
        "/etc",
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
        "/etc/fonts",
        "/etc/fonts",
        "--ro-bind-try",
        "/etc/ld.so.cache",
        "/etc/ld.so.cache",
        "--ro-bind-try",
        "/var/cache/fontconfig",
        "/var/cache/fontconfig",
        "--ro-bind-try",
        "/etc/ImageMagick-7",
        "/etc/ImageMagick-7",
        "--ro-bind-try",
        "/etc/ImageMagick-6",
        "/etc/ImageMagick-6",
    ]);
    helper_binary::bind(&mut command, executable)?;
    command.arg("--ro-bind").arg(input).arg(&input_path);
    command.arg("--bind").arg(output).arg("/output");
    if operation.uses_office_renderer() {
        command.args([
            "--symlink",
            "usr/bin",
            "/bin",
            "--setenv",
            "LANG",
            "C.UTF-8",
        ]);
        if Path::new("/opt/libreoffice/program/soffice").is_file() {
            command.args([
                "--dir",
                "/opt",
                "--ro-bind",
                "/opt/libreoffice",
                "/opt/libreoffice",
            ]);
        }
    }
    let address_space = if operation.uses_office_renderer() {
        OFFICE_ADDRESS_SPACE_LIMIT_BYTES
    } else {
        ADDRESS_SPACE_LIMIT_BYTES
    };
    let cpu_time = if operation.uses_office_renderer() {
        OFFICE_CPU_TIME_LIMIT_SECS
    } else {
        CPU_TIME_LIMIT_SECS
    };
    command.args([
        "--",
        "/usr/bin/prlimit",
        &format!("--as={address_space}"),
        &format!("--cpu={cpu_time}"),
        &format!("--fsize={FILE_SIZE_LIMIT_BYTES}"),
    ]);
    if operation.uses_office_renderer() {
        command.arg(format!("--data={OFFICE_DATA_LIMIT_BYTES}"));
    }
    command.args([
        "--",
        "/app/strata",
        "--preview-helper",
        operation.argument(),
        &input_path,
    ]);
    command.arg(format!("/output/{}", operation.output_name()));
    command.arg(value.to_string());
    Ok(command)
}

fn sandbox_input_path(input: &Path) -> String {
    match input.extension().and_then(|extension| extension.to_str()) {
        Some(extension)
            if !extension.is_empty()
                && extension.len() <= 16
                && extension.bytes().all(|character| character.is_ascii_alphanumeric()) =>
        {
            format!("/input.{}", extension.to_ascii_lowercase())
        }
        _ => "/input".to_owned(),
    }
}

fn terminate(child: &mut std::process::Child) {
    // bwrap is the PID-namespace init process. Killing it tears down every process in
    // the sandbox, including descendants started by ImageMagick or thumbnail tools.
    let _killed = child.kill();
    let _waited = child.wait();
}

fn read_metadata(path: &Path) -> (i32, i32) {
    let Ok(value) = fs::read_to_string(path) else {
        return (0, 0);
    };
    let mut values = value
        .split_whitespace()
        .filter_map(|part| part.parse().ok());
    (values.next().unwrap_or(0), values.next().unwrap_or(0))
}

struct PrivateOutput(PathBuf);

impl PrivateOutput {
    fn create() -> io::Result<Self> {
        use std::os::unix::fs::DirBuilderExt;

        let path = std::env::temp_dir().join(format!(
            "strata-preview-{}-{}",
            std::process::id(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        fs::DirBuilder::new().mode(0o700).create(&path)?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for PrivateOutput {
    fn drop(&mut self) {
        let _removed = fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests;
