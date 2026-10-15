use gtk::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(default)]
pub(super) struct Options {
    pub open_with: bool,
    pub preview: bool,
    pub pin: bool,
    pub transfer_destinations: bool,
    pub archives: bool,
    pub compress_files: bool,
    pub music: bool,
    pub custom_actions: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            open_with: true,
            preview: true,
            pin: true,
            transfer_destinations: false,
            archives: true,
            compress_files: false,
            music: true,
            custom_actions: true,
        }
    }
}

pub(super) fn tidy_separators(container: &gtk::Box) {
    let mut child = container.first_child();
    let mut preceding_item = false;
    let mut pending_separator = None::<gtk::Widget>;
    while let Some(widget) = child {
        child = widget.next_sibling();
        if widget.is::<gtk::Separator>() {
            widget.set_visible(false);
            if preceding_item && pending_separator.is_none() {
                pending_separator = Some(widget);
            }
        } else if widget.get_visible() {
            if let Some(separator) = pending_separator.take() {
                separator.set_visible(true);
            }
            preceding_item = true;
        }
    }
}

#[cfg(test)]
mod tests;
