use std::{
    fs::File,
    path::Path,
    process::{Command, Stdio},
};

use super::sniff;

pub(super) fn render(path: &Path, size: i32) -> Result<Vec<u8>, String> {
    render_with("vips", path, size)
}

fn render_with(executable: &str, path: &Path, size: i32) -> Result<Vec<u8>, String> {
    if !(16..=1400).contains(&size) {
        return Err("Invalid scaled preview size".into());
    }
    let input = File::open(path).map_err(|error| error.to_string())?;
    if !input
        .metadata()
        .is_ok_and(|metadata| metadata.is_file() && metadata.len() <= 128 * 1024 * 1024)
    {
        return Err("Scaled preview input exceeds the file budget".into());
    }
    let output = Command::new(executable)
        .args([
            "--vips-concurrency=2",
            "--vips-cache-max=0",
            "thumbnail_source",
            "[descriptor=0]",
            ".png[compression=1,filter=none]",
        ])
        .arg(size.to_string())
        .arg("--height")
        .arg(size.to_string())
        .args(["--size", "down", "--fail-on", "error"])
        .stdin(Stdio::from(input))
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() || !valid_output(&output.stdout, size) {
        return Err("Optional scaled decoder could not render this image".into());
    }
    Ok(output.stdout)
}

fn valid_output(bytes: &[u8], size: i32) -> bool {
    (16..=1400).contains(&size)
        && bytes.len() <= crate::sandbox::MAX_OUTPUT_BYTES as usize
        && sniff::sniff_png_dimensions(bytes).is_some_and(|dimensions| {
            dimensions.width > 0
                && dimensions.height > 0
                && dimensions.width.max(dimensions.height) <= size as u32
                && !sniff::exceeds_decoded_frame_budget(dimensions.width, dimensions.height)
        })
}

#[cfg(test)]
mod tests;
