use super::*;

pub(super) fn supports_layout(entry: &FileEntry) -> bool {
    entry.location.native_path().is_some()
        && std::path::Path::new(&entry.native_name)
            .extension()
            .and_then(std::ffi::OsStr::to_str)
            .is_some_and(|extension| {
                extension.eq_ignore_ascii_case("xlsx") || extension.eq_ignore_ascii_case("ods")
            })
}

impl PreviewState {
    pub(super) fn render_workbook_controls(self: &Rc<Self>, entry: &FileEntry, layout: bool) {
        let controls = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        controls.add_css_class("document-view-switch");
        let layout_button = gtk::ToggleButton::with_label("Layout");
        let data_button = gtk::ToggleButton::with_label("Data");
        data_button.set_group(Some(&layout_button));
        layout_button.set_active(layout);
        data_button.set_active(!layout);
        layout_button.set_tooltip_text(Some(
            "Rendered print layout with original formatting; read-only",
        ));
        data_button.set_tooltip_text(Some("Sortable worksheet data; read-only and bounded"));
        let available = supports_layout(entry) && crate::sandbox::office::available();
        layout_button.set_sensitive(available);
        if !available {
            layout_button.set_tooltip_text(Some(if supports_layout(entry) {
                "Install LibreOffice to render the workbook layout"
            } else {
                "Rendered layouts support local XLSX and ODS workbooks"
            }));
        }
        for (button, next_layout) in [(&layout_button, true), (&data_button, false)] {
            button.add_css_class("document-view-button");
            let state = Rc::downgrade(self);
            let entry = entry.clone();
            button.connect_clicked(move |button| {
                let Some(state) = state.upgrade() else {
                    return;
                };
                if button.is_active() && state.workbook_layout.get() != next_layout {
                    state.layout_failure.borrow_mut().take();
                    state.load_document(entry.clone(), 0, next_layout);
                }
            });
            controls.append(button);
        }
        self.content.append(&controls);
        let note = gtk::Label::new(Some(if layout {
            "Printed layout · follows workbook print settings. Some charts or embedded objects may be omitted for safety."
        } else if self.layout_failure.borrow().is_some() {
            "Layout could not be rendered. Showing worksheet data instead."
        } else {
            "Worksheet data · original formatting is available in Layout"
        }));
        note.add_css_class("preview-note");
        note.set_xalign(0.0);
        note.set_wrap(true);
        if let Some(reason) = self.layout_failure.borrow().as_ref() {
            note.set_tooltip_text(Some(reason));
        }
        self.content.append(&note);
    }
}

#[cfg(test)]
mod tests;
