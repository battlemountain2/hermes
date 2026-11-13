use super::*;

impl PreviewState {
    pub(super) fn show_cached_image(self: &Rc<Self>, entry: &FileEntry, request: PreviewRequestId) {
        let office = matches!(
            classify_by_name(&entry.native_name),
            FormatFamily::OfficeDocument | FormatFamily::Spreadsheet
        );
        if classify_by_name(&entry.native_name) == FormatFamily::Spreadsheet
            && !self.workbook_layout.get()
        {
            return;
        }
        if !matches!(
            classify_by_name(&entry.native_name),
            FormatFamily::Image
                | FormatFamily::Heif
                | FormatFamily::RawImage
                | FormatFamily::Svg
                | FormatFamily::GeoTiff
                | FormatFamily::OfficeDocument
                | FormatFamily::Spreadsheet
        ) || crate::services::is_gif_image(&entry.native_name, "")
        {
            return;
        }
        let Some(path) = entry.location.native_path().map(ToOwned::to_owned) else {
            return;
        };
        let state = Rc::downgrade(self);
        let cancellation = crate::sandbox::Cancellation::default();
        let worker_cancellation = cancellation.clone();
        let task = glib::MainContext::default().spawn_local(async move {
            let cached = super::super::thumbnail::cached_preview_bytes(&path).await;
            let saved = cached.is_none() && office;
            let bytes = if let Some(cached) = cached {
                Some(cached)
            } else if office {
                let source = path.clone();
                gio::spawn_blocking(move || {
                    crate::sandbox::office::poster(&source, &worker_cancellation)
                })
                .await
                .ok()
                .and_then(Result::ok)
                .map(glib::Bytes::from_owned)
            } else {
                None
            };
            let Some(bytes) = bytes else {
                return;
            };
            let Some(state) = state
                .upgrade()
                .filter(|state| state.current_request.get() == Some(request))
            else {
                return;
            };
            let Some(entry) = state.current.borrow().clone() else {
                return;
            };
            if crate::adapters::LocalPreviewProvider::check_policy(&entry, None).is_err() {
                return;
            }
            let Ok(texture) = gtk::gdk::Texture::from_bytes(&bytes) else {
                return;
            };
            let loading = image::ImagePreview::new_loading(&texture);
            loading.widget.add_css_class("preview-loading-image");
            loading.widget.set_sensitive(false);
            let progress = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            progress.add_css_class("preview-loading-progress");
            progress.set_halign(gtk::Align::End);
            progress.set_valign(gtk::Align::End);
            progress.set_margin_end(12);
            progress.set_margin_bottom(12);
            let spinner = gtk::Spinner::new();
            spinner.start();
            progress.append(&spinner);
            progress.append(&gtk::Label::new(Some(if office {
                if saved {
                    "Saved preview · preparing document…"
                } else {
                    "Cached preview · preparing document…"
                }
            } else {
                "Loading full preview…"
            })));
            if let Some(viewport) = loading.widget.first_child().and_downcast::<gtk::Overlay>() {
                viewport.add_overlay(&progress);
            }
            clear_box(&state.content);
            if classify_by_name(&entry.native_name) == FormatFamily::Spreadsheet {
                state.render_workbook_controls(&entry, state.workbook_layout.get());
            }
            state.content.append(&loading.widget);
            state.loading_image.replace(Some(loading));
        });
        self.loading_task.replace(Some(LoadHandle::new(move || {
            cancellation.cancel();
            task.abort();
        })));
    }
}

#[cfg(test)]
mod tests;
