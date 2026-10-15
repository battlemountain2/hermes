use super::{SearchEntityKey, SearchGroup};
use gio::prelude::FileExt;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum SearchScope {
    ThisFolder,
    #[default]
    Home,
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
    pub(crate) group: SearchGroup,
    pub(crate) entity: Option<SearchEntityKey>,
}

impl ScopedSearch {
    pub(crate) fn new(scope: SearchScope, current_folder: &Path, query: &str) -> Self {
        Self {
            scope,
            root: scope.root(current_folder),
            query: query.trim().to_owned(),
            group: SearchGroup::All,
            entity: None,
        }
    }

    pub(crate) fn with_group(
        mut self,
        group: SearchGroup,
        entity: Option<SearchEntityKey>,
    ) -> Self {
        self.group = group;
        self.entity = entity;
        self
    }

    pub(crate) fn to_uri(&self) -> String {
        let mut uri = format!(
            "search://?scope={}&root={}&q={}",
            self.scope.name(),
            gio::glib::uri_escape_string(
                gio::File::for_path(&self.root).uri(),
                None::<&str>,
                false
            ),
            gio::glib::uri_escape_string(&self.query, None::<&str>, false)
        );
        if self.group != SearchGroup::All {
            uri.push_str("&group=");
            uri.push_str(match self.group {
                SearchGroup::All => "all",
                SearchGroup::Files => "files",
                SearchGroup::Songs => "songs",
                SearchGroup::Albums => "albums",
                SearchGroup::Artists => "artists",
            });
        }
        if let Some(entity) = &self.entity {
            let (artist, album) = match entity {
                SearchEntityKey::Album { artist, album } => (artist, Some(album)),
                SearchEntityKey::Artist(artist) => (artist, None),
            };
            uri.push_str("&artist-group=");
            uri.push_str(&gio::glib::uri_escape_string(artist, None::<&str>, false));
            if let Some(album) = album {
                uri.push_str("&album-group=");
                uri.push_str(&gio::glib::uri_escape_string(album, None::<&str>, false));
            }
        }
        uri
    }

    pub(crate) fn from_uri(uri: &str) -> Result<Self, String> {
        let body = uri
            .strip_prefix("search://")
            .ok_or("Not a search location")?;
        if !body.starts_with('?') {
            return Ok(Self::new(SearchScope::Home, Path::new("/"), body));
        }
        let mut scope = None;
        let mut root = None;
        let mut query = None;
        let mut group = None;
        let mut entity_artist = None;
        let mut entity_album = None;
        for field in body[1..].split('&') {
            let (key, encoded) = field.split_once('=').ok_or("Invalid search scope")?;
            let value = gio::glib::uri_unescape_string(encoded, None::<&str>)
                .ok_or("Invalid search encoding")?
                .to_string();
            match key {
                "scope" if scope.is_none() => {
                    scope = Some(match value.as_str() {
                        "folder" => SearchScope::ThisFolder,
                        "home" => SearchScope::Home,
                        "music" => SearchScope::MusicLibrary,
                        _ => return Err("Unknown search scope".into()),
                    })
                }
                "root" if root.is_none() => {
                    root = Some(if value.starts_with("file:") {
                        gio::File::for_uri(&value)
                            .path()
                            .ok_or("Search root must be local")?
                    } else {
                        PathBuf::from(value)
                    })
                }
                "q" if query.is_none() => query = Some(value),
                "group" if group.is_none() => {
                    group = Some(match value.as_str() {
                        "all" => SearchGroup::All,
                        "files" => SearchGroup::Files,
                        "songs" => SearchGroup::Songs,
                        _ => {
                            return Err(
                                "Open an album or artist before showing its songs in a tab".into(),
                            );
                        }
                    })
                }
                "artist-group" if entity_artist.is_none() => {
                    entity_artist = Some(entity_field(value)?)
                }
                "album-group" if entity_album.is_none() => {
                    entity_album = Some(entity_field(value)?)
                }
                _ => return Err("Invalid or duplicate search field".into()),
            }
        }
        let scope = scope.ok_or("Missing search scope")?;
        let root = root.ok_or("Missing search root")?;
        if !root.is_absolute() {
            return Err("Search root must be absolute".into());
        }
        let resolved = scope.root(&root);
        if resolved != root {
            return Err("Search root does not match its scope".into());
        }
        let query = query.ok_or("Missing search query")?;
        if query.len() > 4096 {
            return Err("Search query exceeds 4096 bytes".into());
        }
        let group = group.unwrap_or(SearchGroup::All);
        let entity = match (entity_artist, entity_album) {
            (Some(artist), Some(album)) if !album.trim().is_empty() => {
                Some(SearchEntityKey::Album { artist, album })
            }
            (Some(artist), None) if !artist.trim().is_empty() => {
                Some(SearchEntityKey::Artist(artist))
            }
            (None, None) => None,
            _ => return Err("Invalid music group".into()),
        };
        if entity.is_some() && group != SearchGroup::Songs {
            return Err("Music groups must open as song results".into());
        }
        Ok(Self {
            scope,
            root,
            query: query.trim().to_owned(),
            group,
            entity,
        })
    }
}

fn entity_field(value: String) -> Result<String, String> {
    let value = value.to_lowercase();
    if value.len() > 2048 || value.chars().count() > 512 || value.chars().any(char::is_control) {
        return Err("Invalid music group name".into());
    }
    Ok(value)
}

#[cfg(test)]
mod tests;
