use super::*;

#[cfg(test)]
mod tests;

pub(super) fn install(
    overlay: &gtk::Overlay,
    root: &BlurBin,
    search: &SearchDialog,
    sidebar: &SidebarView,
) -> gtk::Box {
    let view = sidebar.state.view.clone();
    let opened = Rc::new(move |uri: String| {
        view.navigate_location(Location::uri(uri));
    });
    let dismissed_root = root.clone();
    let dismissed = Rc::new(move || dismissed_root.set_blurred(false));
    let dialog = super::super::saved_searches::SavedSearchDialog::new(
        sidebar.state.saved_searches.clone(),
        opened,
        dismissed,
    );
    let layer = dialog.widget();
    overlay.add_overlay(&layer);
    let visible_root = root.clone();
    layer.connect_visible_notify(move |layer| visible_root.set_blurred(layer.is_visible()));
    let saved_dialog = dialog.clone();
    search.set_save_search(Rc::new(move |uri| saved_dialog.show_create(&uri)));
    sidebar
        .state
        .manage_saved_searches
        .replace(Some(Rc::new(move || dialog.show_manage())));
    sidebar.state.rebuild();
    layer
}

impl SidebarState {
    pub(super) fn refresh_saved_searches(self: &Rc<Self>) {
        let revision = self.saved_searches.revision();
        if revision == self.saved_revision.get() {
            return;
        }
        if self.theme_manager.show_smart_folders() {
            self.rebuild();
        } else {
            self.saved_revision.set(revision);
        }
    }

    pub(super) fn append_smart_folders(self: &Rc<Self>) {
        let snapshot = self.saved_searches.snapshot();
        self.saved_revision.set(snapshot.revision);
        self.append_separator();
        self.append_heading("SMART FOLDERS");
        for saved in snapshot.entries {
            let row = sidebar_button(crate::assets::icons::SEARCH, &saved.name);
            row.set_tooltip_text(Some(
                "Saved search · shows current matches without moving files",
            ));
            let location = Location::uri(saved.uri);
            self.place_rows
                .borrow_mut()
                .push((location.clone(), row.clone()));
            let weak = Rc::downgrade(self);
            row.connect_clicked(move |_| {
                if let Some(state) = weak.upgrade() {
                    state.view.navigate_location(location.clone());
                }
            });
            self.widget.append(&row);
        }
        let manage = sidebar_button(crate::assets::icons::SLIDERS, "Manage searches…");
        manage.set_tooltip_text(Some(
            "Open, rename, edit or remove saved searches; your files stay untouched",
        ));
        let weak = Rc::downgrade(self);
        manage.connect_clicked(move |_| {
            if let Some(state) = weak.upgrade() {
                let callback = state.manage_saved_searches.borrow().clone();
                if let Some(callback) = callback {
                    callback();
                }
            }
        });
        self.widget.append(&manage);
    }
}
