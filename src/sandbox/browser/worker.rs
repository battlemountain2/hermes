// SPDX-License-Identifier: GPL-3.0-or-later

use std::{
    fs::File,
    io::Write,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    os::unix::net::UnixStream,
    path::Path,
};

use super::{
    process,
    wire::{self, Operation, Response},
};

pub(crate) fn run() -> Result<(), String> {
    // FD 0 was passed as the UnixStream connected to the host
    #[expect(
        unsafe_code,
        reason = "Takes ownership of inherited stdin file descriptor 0 as UnixStream"
    )]
    // SAFETY: FD 0 was initialized by the host process as the connected UnixStream
    let mut socket = unsafe { UnixStream::from_raw_fd(0) };

    loop {
        match wire::receive(&socket) {
            Ok(Some((operation, input_fd, output_fd))) => {
                let success = handle_request(operation, input_fd, output_fd);
                let status_byte = if success { 1u8 } else { 0u8 };
                if socket.write_all(&[status_byte]).is_err() {
                    break;
                }
            }
            Ok(None) => {
                // Host closed connection
                break;
            }
            Err(error) => {
                eprintln!("Browser worker receive error: {error}");
                break;
            }
        }
    }

    Ok(())
}

fn handle_request(operation: Operation, input_fd: OwnedFd, output_fd: OwnedFd) -> bool {
    // Try forking a disposable child process for pristine memory isolation
    match process::fork_child() {
        Ok(Some(child_pid)) => {
            // Parent: wait for child
            process::wait(child_pid).unwrap_or(false)
        }
        Ok(None) => {
            // Child: execute render and write response
            let result = execute_job(operation, &input_fd, &output_fd);
            process::exit(result);
        }
        Err(_) => {
            // Fork failed (e.g. running in restrictive environment): execute in-process
            execute_job(operation, &input_fd, &output_fd).is_ok()
        }
    }
}

fn execute_job(
    operation: Operation,
    input_fd: &OwnedFd,
    output_fd: &OwnedFd,
) -> Result<(), String> {
    let (response, error) = match render_job(operation, input_fd) {
        Ok(response) => (response, None),
        Err(error) => (Response::failure(&error), Some(error)),
    };
    let mut writer = File::from(output_fd.try_clone().map_err(|error| error.to_string())?);
    response
        .write(&mut writer)
        .map_err(|error| error.to_string())?;
    error.map_or(Ok(()), Err)
}

fn render_job(operation: Operation, input_fd: &OwnedFd) -> Result<Response, String> {
    let input_path_str = format!("/proc/self/fd/{}", input_fd.as_raw_fd());
    let input_path = Path::new(&input_path_str);

    let (png, metadata) = match operation {
        Operation::ImageMetadata => (crate::sandbox_helper::metadata::image(input_path)?, None),
        Operation::MediaMetadata => (crate::sandbox_helper::metadata::media(input_path)?, None),
        Operation::MusicTags => (
            crate::sandbox_helper::audio::read_music_tags(input_path)?.into_bytes(),
            None,
        ),
        Operation::PreviewAudioMetadata => (
            crate::sandbox_helper::audio::read_audio_metadata(input_path)?.into_bytes(),
            None,
        ),
        Operation::Image => (crate::sandbox_helper::render_image(input_path, 256)?, None),
        Operation::Video => (crate::sandbox_helper::render_media(input_path, 256)?, None),
        Operation::PreviewImage => (crate::sandbox_helper::render_image(input_path, 1400)?, None),
        Operation::PreviewAudioArtwork => (crate::sandbox_helper::render_media(input_path, 1024)?, None),
        Operation::Heif => (
            crate::sandbox_helper::render_heif_thumbnail(input_path, 256)?,
            None,
        ),
        Operation::PreviewHeif => (
            crate::sandbox_helper::render_heif_preview(input_path)?,
            None,
        ),
        Operation::PreviewGeoTiff => {
            let (png, meta_json) =
                crate::sandbox_helper::geotiff::render_geotiff(input_path, 1400)?;
            (png, Some(meta_json.into_bytes()))
        }
        Operation::Raw => (crate::sandbox_helper::render_raw(input_path, 256)?, None),
        Operation::Pdf => (crate::sandbox_helper::render_pdf_thumbnail(input_path, 256)?, None),
        Operation::PreviewModel => (
            crate::sandbox_helper::model::render_model_with_format(
                input_path,
                800,
                crate::services::model_preview::ModelFormat::Stl,
            )?,
            None,
        ),
        Operation::PreviewThreeMf => (
            crate::sandbox_helper::model::render_model_with_format(
                input_path,
                800,
                crate::services::model_preview::ModelFormat::ThreeMf,
            )?,
            None,
        ),
        Operation::PreviewEpubCover
        | Operation::PreviewCbzCover
        | Operation::PreviewCbrCover
        | Operation::PreviewPptxCover => {
            use crate::sandbox_helper::archive_cover::CoverFormat;
            let format = match operation {
                Operation::PreviewEpubCover => CoverFormat::Epub,
                Operation::PreviewCbzCover => CoverFormat::Cbz,
                Operation::PreviewCbrCover => CoverFormat::Cbr,
                Operation::PreviewPptxCover => CoverFormat::Pptx,
                _ => unreachable!(),
            };
            (
                crate::sandbox_helper::archive_cover::render_cover_with_format(
                    input_path, 1400, format,
                )?,
                None,
            )
        }
        Operation::PreviewArchiveCover => {
            return Err("Archive cover request requires an explicit format".into());
        }
        Operation::PreviewSpreadsheet => {
            let (png, json) = crate::sandbox_helper::table::render_spreadsheet(input_path)?;
            (png, Some(json.into_bytes()))
        }
        Operation::PreviewAudioWaveform => {
            let (png, meta_json) =
                crate::sandbox_helper::audio::render_waveform_and_meta(input_path)?;
            (png, Some(meta_json.into_bytes()))
        }
        _ => (crate::sandbox_helper::render_image(input_path, 256)?, None),
    };

    Ok(Response {
        png,
        metadata: metadata.unwrap_or_default(),
    })
}

#[cfg(test)]
mod format_tests;
