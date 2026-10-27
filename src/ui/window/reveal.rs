use std::{cell::Cell, rc::Rc};

use crate::{
    app::{Browser, BrowserEvent},
    file_manager::Request,
};

use super::BrowserView;

pub(super) fn includes_hidden(request: &Request) -> bool {
    use gtk::{gio, prelude::*};
    request.selection.iter().any(|location| {
        let file = location
            .native_path()
            .map(gio::File::for_path)
            .or_else(|| location.uri_value().map(gio::File::for_uri));
        file.and_then(|file| file.basename())
            .is_some_and(|name| name.as_os_str().as_encoded_bytes().starts_with(b"."))
    })
}

pub(super) fn install(browser: &Rc<Browser>, view: &BrowserView, request: Request) {
    if request.selection.is_empty() {
        if request.properties {
            view.show_location_properties(&request.directory);
        }
        return;
    }
    let complete = Rc::new(Cell::new(false));
    let weak_browser = Rc::downgrade(browser);
    let selected_view = view.clone();
    let apply: Rc<dyn Fn()> = Rc::new(move || {
        if complete.get() {
            return;
        }
        let Some(browser) = weak_browser.upgrade() else {
            return;
        };
        let Some(depth) = browser.active_depth() else {
            return;
        };
        let Some(column) = browser.column_snapshot(depth) else {
            return;
        };
        if column.location != request.directory {
            complete.set(true);
            return;
        }
        if column.loading {
            return;
        }
        complete.set(true);
        if browser.select_locations(depth, &request.selection) && request.properties {
            selected_view.show_focused_properties();
        }
    });
    let observed = apply.clone();
    browser.observe(move |event| {
        if matches!(
            event,
            BrowserEvent::LoadFinished { .. }
                | BrowserEvent::TabRestored { .. }
                | BrowserEvent::Reset
                | BrowserEvent::ColumnsTruncated { .. }
        ) {
            observed();
        }
    });
    apply();
}

#[cfg(test)]
mod tests;
