use std::path::{Path, PathBuf};
use gio::prelude::FileExt;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum SearchScope {
    ThisFolder,
    Home,
    #[default]
    MusicLibrary,
}

impl SearchScope {
    pub(crate) fn root(self, current_folder: &Path) -> PathBuf {
        match self {
            Self::ThisFolder => current_folder.to_path_buf(),
            Self::Home => gio::glib::home_dir(),
            Self::MusicLibrary => gio::glib::user_special_dir(gio::glib::UserDirectory::Music)
                .unwrap_or_else(|| gio::glib::home_dir().join("Music")),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::ThisFolder => "folder",
            Self::Home => "home",
            Self::MusicLibrary => "music",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ScopedSearch {
    pub(crate) scope: SearchScope,
    pub(crate) root: PathBuf,
    pub(crate) query: String,
}

impl ScopedSearch {
    pub(crate) fn new(scope: SearchScope, current_folder: &Path, query: &str) -> Self {
        Self { scope, root: scope.root(current_folder), query: query.trim().to_owned() }
    }

    pub(crate) fn to_uri(&self) -> String {
        format!("search://?scope={}&root={}&q={}", self.scope.name(),
            gio::glib::uri_escape_string(gio::File::for_path(&self.root).uri(), None::<&str>, false),
            gio::glib::uri_escape_string(&self.query, None::<&str>, false))
    }

    pub(crate) fn from_uri(uri: &str) -> Result<Self, String> {
        let body = uri.strip_prefix("search://").ok_or("Not a search location")?;
        if !body.starts_with('?') {
            return Ok(Self::new(SearchScope::Home, Path::new("/"), body));
        }
        let mut scope = None;
        let mut root = None;
        let mut query = None;
        for field in body[1..].split('&') {
            let (key, encoded) = field.split_once('=').ok_or("Invalid search scope")?;
            let value = gio::glib::uri_unescape_string(encoded, None::<&str>).ok_or("Invalid search encoding")?.to_string();
            match key {
                "scope" if scope.is_none() => scope = Some(match value.as_str() {
                    "folder" => SearchScope::ThisFolder,
                    "home" => SearchScope::Home,
                    "music" => SearchScope::MusicLibrary,
                    _ => return Err("Unknown search scope".into()),
                }),
                "root" if root.is_none() => root = Some(if value.starts_with("file:") {
                    gio::File::for_uri(&value).path().ok_or("Search root must be local")?
                } else { PathBuf::from(value) }),
                "q" if query.is_none() => query = Some(value),
                _ => return Err("Invalid or duplicate search field".into()),
            }
        }
        let scope = scope.ok_or("Missing search scope")?;
        let root = root.ok_or("Missing search root")?;
        if !root.is_absolute() { return Err("Search root must be absolute".into()); }
        let resolved = scope.root(&root);
        if resolved != root { return Err("Search root does not match its scope".into()); }
        let query = query.ok_or("Missing search query")?;
        if query.len() > 4096 { return Err("Search query exceeds 4096 bytes".into()); }
        Ok(Self { scope, root, query: query.trim().to_owned() })
    }
}
