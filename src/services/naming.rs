// SPDX-License-Identifier: GPL-3.0-or-later

#[cfg(test)]
mod tests;

/// Generates a collision-safe copy of a filename.
///
/// Given `photo.jpg`, produces `photo (copy).jpg`, then `photo (copy 2).jpg`, etc.
/// For directories or extensionless files: `folder` → `folder (copy)` → `folder (copy 2)`.
/// The `exists` closure checks whether a candidate name already exists in the target directory.
pub fn collision_safe_name(basename: &str, exists: &dyn Fn(&str) -> bool) -> String {
    let (stem, ext) = split_stem_extension(basename);
    let first = if ext.is_empty() {
        format!("{stem} (copy)")
    } else {
        format!("{stem} (copy).{ext}")
    };
    if !exists(&first) {
        return first;
    }
    for n in 2..=10_000 {
        let candidate = if ext.is_empty() {
            format!("{stem} (copy {n})")
        } else {
            format!("{stem} (copy {n}).{ext}")
        };
        if !exists(&candidate) {
            return candidate;
        }
    }
    // Fallback: return the last candidate tried
    if ext.is_empty() {
        format!("{stem} (copy 10000)")
    } else {
        format!("{stem} (copy 10000).{ext}")
    }
}

/// Splits a filename into stem and extension.
///
/// - `photo.jpg` → (`photo`, `jpg`)
/// - `archive.tar.gz` → (`archive.tar`, `gz`)
/// - `Makefile` → (`Makefile`, ``)
/// - `.gitignore` → (`.gitignore`, ``)
/// - `.bashrc` → (`.bashrc`, ``)
fn split_stem_extension(name: &str) -> (&str, &str) {
    // Dotfiles with no further extension: .gitignore, .bashrc
    if name.starts_with('.') {
        match name[1..].rfind('.') {
            Some(pos) => (&name[..pos + 1], &name[pos + 2..]),
            None => (name, ""),
        }
    } else {
        match name.rfind('.') {
            Some(0) => (name, ""),
            Some(pos) => (&name[..pos], &name[pos + 1..]),
            None => (name, ""),
        }
    }
}
