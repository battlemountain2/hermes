// SPDX-License-Identifier: GPL-3.0-or-later

use std::{ffi::OsStr, path::Path, rc::Rc};

use crate::model::FileEntry;

use super::LoadHandle;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PreviewRequestId(pub u64);

#[derive(Clone, Debug)]
pub struct PreviewRequest {
    pub id: PreviewRequestId,
    pub entry: FileEntry,
    pub text_byte_limit: usize,
    pub pdf_page: i32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PreviewContent {
    Text { content: String, truncated: bool },
    Image,
    Media,
    Rasterized { png: Vec<u8> },
    SandboxedMedia { data: Vec<u8> },
    Pdf { png: Vec<u8>, page: i32, pages: i32 },
    Unsupported,
}

#[derive(Clone, Debug)]
pub struct Preview {
    pub request_id: PreviewRequestId,
    pub entry: FileEntry,
    pub content_type: String,
    pub content: PreviewContent,
}

#[derive(Clone, Debug)]
pub enum PreviewEvent {
    Ready(Preview),
    Failed {
        request_id: PreviewRequestId,
        entry: FileEntry,
        message: String,
    },
}

pub trait PreviewProvider {
    fn load(&self, request: PreviewRequest, emit: Rc<dyn Fn(PreviewEvent)>) -> LoadHandle;
}

pub(crate) fn has_plain_text_extension(name: &OsStr) -> bool {
    let path = Path::new(name);
    if path
        .file_name()
        .and_then(OsStr::to_str)
        .is_some_and(|name| {
            matches!(
                name.to_ascii_lowercase().as_str(),
                "dockerfile" | "gemfile" | "makefile" | "meson.build"
            )
        })
    {
        return true;
    }
    path.extension()
        .and_then(OsStr::to_str)
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "bash"
                    | "c"
                    | "cc"
                    | "conf"
                    | "cpp"
                    | "cs"
                    | "css"
                    | "csv"
                    | "desktop"
                    | "fish"
                    | "go"
                    | "h"
                    | "hpp"
                    | "htm"
                    | "html"
                    | "ini"
                    | "java"
                    | "js"
                    | "json"
                    | "jsonl"
                    | "jsx"
                    | "kt"
                    | "kts"
                    | "less"
                    | "log"
                    | "lua"
                    | "md"
                    | "markdown"
                    | "py"
                    | "rb"
                    | "rs"
                    | "rst"
                    | "scss"
                    | "service"
                    | "sh"
                    | "sql"
                    | "svg"
                    | "toml"
                    | "ts"
                    | "tsv"
                    | "tsx"
                    | "txt"
                    | "xml"
                    | "yaml"
                    | "yml"
                    | "zsh"
            )
        })
}

pub(crate) fn content_family(content_type: &str) -> PreviewContent {
    if content_type == "application/pdf" {
        PreviewContent::Pdf {
            png: Vec::new(),
            page: 0,
            pages: 0,
        }
    } else if content_type.starts_with("image/") {
        PreviewContent::Image
    } else if content_type.starts_with("audio/") || content_type.starts_with("video/") {
        PreviewContent::Media
    } else if content_type.starts_with("text/")
        || matches!(
            content_type,
            "application/json"
                | "application/ld+json"
                | "application/toml"
                | "application/x-yaml"
                | "application/xml"
                | "application/javascript"
                | "application/x-javascript"
                | "application/x-shellscript"
        )
        || content_type.ends_with("+json")
        || content_type.ends_with("+xml")
    {
        PreviewContent::Text {
            content: String::new(),
            truncated: false,
        }
    } else {
        PreviewContent::Unsupported
    }
}

#[cfg(test)]
mod tests;
