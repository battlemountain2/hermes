// SPDX-License-Identifier: GPL-3.0-or-later

use std::{cmp::Ordering, ffi::OsString, path::PathBuf};

use serde::{Deserialize, Serialize};

/// A browsable destination. Native paths remain byte-safe and URI locations remain explicit.
#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
enum LocationKind {
    #[serde(with = "native_path")]
    Native(PathBuf),
    Uri(String),
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct Location {
    #[serde(flatten)]
    kind: LocationKind,
}

#[cfg(unix)]
mod native_path {
    use std::{
        ffi::OsString,
        os::unix::ffi::OsStringExt,
        path::{Path, PathBuf},
    };

    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S>(path: &Path, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        path.as_os_str().as_encoded_bytes().serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<PathBuf, D::Error>
    where
        D: Deserializer<'de>,
    {
        Vec::<u8>::deserialize(deserializer).map(|bytes| PathBuf::from(OsString::from_vec(bytes)))
    }
}

impl Location {
    pub fn local(path: impl Into<PathBuf>) -> Self {
        Self {
            kind: LocationKind::Native(path.into()),
        }
    }

    pub fn uri(uri: impl Into<String>) -> Self {
        Self {
            kind: LocationKind::Uri(uri.into()),
        }
    }

    pub fn native_path(&self) -> Option<&std::path::Path> {
        match &self.kind {
            LocationKind::Native(path) => Some(path),
            LocationKind::Uri(_) => None,
        }
    }

    pub fn uri_value(&self) -> Option<&str> {
        match &self.kind {
            LocationKind::Native(_) => None,
            LocationKind::Uri(uri) => Some(uri),
        }
    }

    pub fn parent(&self) -> Option<Self> {
        let path = self.native_path()?;
        let parent = path.parent()?;
        (parent != path).then(|| Self::local(parent))
    }

    pub fn is_absolute_native(&self) -> bool {
        self.native_path().is_some_and(std::path::Path::is_absolute)
    }

    pub fn rebase(&self, from: &Self, to: &Self) -> Option<Self> {
        let suffix = self.native_path()?.strip_prefix(from.native_path()?).ok()?;
        Some(Self::local(to.native_path()?.join(suffix)))
    }

    pub fn is_within(&self, other: &Self) -> bool {
        self.native_path()
            .zip(other.native_path())
            .is_some_and(|(path, parent)| path.starts_with(parent))
    }

    pub fn compare(&self, other: &Self) -> Ordering {
        match (&self.kind, &other.kind) {
            (LocationKind::Native(left), LocationKind::Native(right)) => left.cmp(right),
            (LocationKind::Uri(left), LocationKind::Uri(right)) => left.cmp(right),
            (LocationKind::Native(_), LocationKind::Uri(_)) => Ordering::Less,
            (LocationKind::Uri(_), LocationKind::Native(_)) => Ordering::Greater,
        }
    }

    /// Returns a UTF-8-safe representation without changing the native path.
    pub fn display_path(&self) -> String {
        match &self.kind {
            LocationKind::Native(path) => path.to_string_lossy().into_owned(),
            LocationKind::Uri(uri) => uri.clone(),
        }
    }

    pub fn display_name(&self) -> String {
        match &self.kind {
            LocationKind::Native(path) => path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| path.to_string_lossy().into_owned()),
            LocationKind::Uri(uri) if uri == "trash:///" => "Trash".into(),
            LocationKind::Uri(uri) => uri
                .trim_end_matches('/')
                .rsplit('/')
                .next()
                .unwrap_or(uri)
                .into(),
        }
    }

    pub fn breadcrumbs(&self) -> Vec<Self> {
        let Some(path) = self.native_path() else {
            return vec![self.clone()];
        };
        let mut locations: Vec<_> = path.ancestors().map(Self::local).collect();
        locations.reverse();
        locations
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SortKey {
    Name,
    Type,
    Size,
    Modified,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SortDirection {
    Ascending,
    Descending,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct ViewPreferences {
    pub show_hidden: bool,
    pub folders_first: bool,
    pub sort_key: SortKey,
    pub sort_direction: SortDirection,
}

mod trail;

pub use trail::{
    BrowserDensity as TrailBrowserDensity, BrowserMode as TrailBrowserMode, Trail, TrailCollection,
    TrailId, TrailViewState,
};

impl Default for ViewPreferences {
    fn default() -> Self {
        Self {
            show_hidden: false,
            folders_first: true,
            sort_key: SortKey::Name,
            sort_direction: SortDirection::Ascending,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum EntryKind {
    Directory,
    DirectorySymbolicLink,
    File,
    FileSymbolicLink,
    SymbolicLink,
    Other,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MetadataValue<T> {
    Unknown,
    Known(T),
    Unavailable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileEntry(pub std::rc::Rc<FileEntryInner>);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileEntryInner {
    pub location: Location,
    pub native_name: OsString,
    pub display_name: String,
    pub kind: EntryKind,
    pub size: MetadataValue<u64>,
    pub modified_unix_seconds: MetadataValue<i64>,
}

impl std::ops::Deref for FileEntry {
    type Target = FileEntryInner;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl FileEntry {
    pub fn is_directory(&self) -> bool {
        matches!(
            self.kind,
            EntryKind::Directory | EntryKind::DirectorySymbolicLink
        )
    }

    pub fn is_symbolic_link(&self) -> bool {
        matches!(
            self.kind,
            EntryKind::DirectorySymbolicLink
                | EntryKind::FileSymbolicLink
                | EntryKind::SymbolicLink
        )
    }

    pub fn is_broken_symbolic_link(&self) -> bool {
        self.kind == EntryKind::SymbolicLink
    }
}

#[cfg(test)]
mod tests;
