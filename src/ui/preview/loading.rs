use super::*;

impl PreviewState {
    pub(super) fn show_cached_image(self: &Rc<Self>, entry: &FileEntry, request: PreviewRequestId) {
        if !matches!(
            classify_by_name(&entry.native_name),
            FormatFamily::Image
                | FormatFamily::Heif
                | FormatFamily::RawImage
                | FormatFamily::Svg
                | FormatFamily::GeoTiff
        ) || crate::services::is_gif_image(&entry.native_name, "")
        {
            return;
        }
        let Some(path) = entry.location.native_path().map(ToOwned::to_owned) else {
            return;
        };
        let state = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let Some(bytes) = super::super::thumbnail::cached_preview_bytes(&path).await else {
                return;
            };
            let Some(state) = state
                .upgrade()
                .filter(|state| state.current_request.get() == Some(request))
            else {
                return;
            };
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
            progress.append(&gtk::Label::new(Some("Loading full preview…")));
            if let Some(viewport) = loading.widget.first_child().and_downcast::<gtk::Overlay>() {
                viewport.add_overlay(&progress);
            }
            clear_box(&state.content);
            state.content.append(&loading.widget);
            state.loading_image.replace(Some(loading));
        });
    }
}

#[cfg(test)]
mod tests;
