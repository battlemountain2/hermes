use super::*;

pub(super) struct Presentation {
    columns: Vec<ColumnView>,
    modes: ModeViews,
    columns_widget: gtk::Box,
    scroller: gtk::ScrolledWindow,
    scroll: Vec<(gtk::Adjustment, f64)>,
}

impl BrowserView {
    pub fn activate_tab(&self, trail: &Trail) {
        let Some(location) = trail.active_location() else {
            return;
        };
        let mode = match trail.view.browser_mode {
            crate::model::TrailBrowserMode::Columns => BrowserMode::Columns,
            crate::model::TrailBrowserMode::Grid => BrowserMode::Grid,
            crate::model::TrailBrowserMode::Explorer => BrowserMode::Explorer,
        };
        let density = match trail.view.density {
            crate::model::TrailBrowserDensity::Compact => BrowserDensity::Compact,
            crate::model::TrailBrowserDensity::Airy => BrowserDensity::Airy,
        };
        self.state.pending_tab_view.set(Some((mode, density)));
        self.state
            .browser
            .activate_tab(trail.id.clone(), location.clone(), trail.view.preferences);
        self.set_view_mode(mode);
        self.set_density(density);
        self.state.pending_tab_view.set(None);
    }
}

impl ViewState {
    pub(super) fn store_presentation(self: &Rc<Self>, id: TrailId) {
        self.cancel_rename();
        self.cancel_new_folder();
        self.close_peek_visual();
        self.peek_anchor.take();
        self.hovered_column.set(None);
        self.horizontal_scroll_generation
            .set(self.horizontal_scroll_generation.get().wrapping_add(1));
        let mut scroll = Vec::new();
        capture_scroll(self.mode_views.borrow().widget().upcast_ref(), &mut scroll);
        let columns = self.columns.take();
        for column in &columns {
            column
                .animation_generation
                .set(column.animation_generation.get().wrapping_add(1));
            column.shell.set_opacity(1.0);
            if let Some(content) = column
                .shell
                .first_child()
                .and_downcast::<gtk::Overlay>()
                .and_then(|overlay| overlay.child())
            {
                content.set_opacity(1.0);
                content.set_margin_start(0);
            }
            self.overlay.remove_overlay(&column.marquee);
        }
        self.mode_views.borrow().cancel_new_folder();
        self.mode_views.borrow().cancel_rename();
        let columns_widget = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        columns_widget.add_css_class("columns");
        columns_widget.set_halign(gtk::Align::Start);
        columns_widget.set_vexpand(true);
        let scroller = gtk::ScrolledWindow::builder()
            .child(&columns_widget)
            .hscrollbar_policy(gtk::PolicyType::Automatic)
            .vscrollbar_policy(gtk::PolicyType::Never)
            .hexpand(true)
            .vexpand(true)
            .build();
        let modes = self.mode_views.borrow().empty_presentation(&scroller);
        self.presentations.add_child(&modes.widget());
        self.clear_tab_focus();
        self.presentations.set_visible_child(&modes.widget());
        let modes = self.mode_views.replace(modes);
        let columns_widget = self.columns_widget.replace(columns_widget);
        let scroller = self.scroller.replace(scroller);
        self.retained_tabs.borrow_mut().push((
            id,
            Presentation {
                columns,
                modes,
                columns_widget,
                scroller,
                scroll,
            },
        ));
    }

    pub(super) fn restore_presentation(self: &Rc<Self>, id: &TrailId) {
        let retained = {
            let mut tabs = self.retained_tabs.borrow_mut();
            tabs.iter()
                .position(|(stored, _)| stored == id)
                .map(|position| tabs.remove(position).1)
        };
        let Some(retained) = retained else {
            return;
        };
        self.truncate(0);
        let old = self.mode_views.replace(retained.modes);
        self.columns_widget.replace(retained.columns_widget);
        self.scroller.replace(retained.scroller);
        self.columns.replace(retained.columns);
        for column in self.columns.borrow().iter() {
            self.overlay.add_overlay(&column.marquee);
        }
        self.clear_tab_focus();
        self.presentations
            .set_visible_child(&self.mode_views.borrow().widget());
        self.presentations.remove(&old.widget());
        self.clear_location_error();
        self.sync_active_location();
        self.refresh_cut_rows();
        self.restore_scroll(retained.scroll);
    }

    pub(super) fn discard_presentation(&self, id: &TrailId) {
        let removed = {
            let mut tabs = self.retained_tabs.borrow_mut();
            tabs.iter()
                .position(|(stored, _)| stored == id)
                .map(|position| tabs.remove(position).1)
        };
        if let Some(presentation) = removed {
            self.presentations.remove(&presentation.modes.widget());
        }
    }

    fn clear_tab_focus(&self) {
        if let Some(root) = self.presentations.root()
            && root
                .focus()
                .is_some_and(|focus| focus.is_ancestor(&self.presentations))
        {
            root.set_focus(None::<&gtk::Widget>);
        }
    }

    fn restore_scroll(self: &Rc<Self>, scroll: Vec<(gtk::Adjustment, f64)>) {
        let generation = self.horizontal_scroll_generation.get();
        for (adjustment, value) in &scroll {
            adjustment.set_value(*value);
        }
        let weak = Rc::downgrade(self);
        let overlay = self.overlay.downgrade();
        if let Some(overlay) = overlay.upgrade() {
            let frames = Cell::new(0);
            overlay.add_tick_callback(move |_, _| {
                if let Some(state) = weak.upgrade()
                    && state.horizontal_scroll_generation.get() == generation
                {
                    for (adjustment, value) in &scroll {
                        adjustment.set_value(*value);
                    }
                    frames.set(frames.get() + 1);
                    if frames.get() < 3 {
                        return glib::ControlFlow::Continue;
                    }
                }
                glib::ControlFlow::Break
            });
        }
    }
}

fn capture_scroll(widget: &gtk::Widget, scroll: &mut Vec<(gtk::Adjustment, f64)>) {
    if let Some(scroller) = widget.downcast_ref::<gtk::ScrolledWindow>() {
        for (adjustment, policy) in [
            (scroller.hadjustment(), scroller.hscrollbar_policy()),
            (scroller.vadjustment(), scroller.vscrollbar_policy()),
        ] {
            if policy != gtk::PolicyType::Never {
                let value = adjustment.value();
                scroll.push((adjustment, value));
            }
        }
    }
    if let Some(stack) = widget.downcast_ref::<gtk::Stack>() {
        if let Some(child) = stack.visible_child() {
            capture_scroll(&child, scroll);
        }
        return;
    }
    let mut child = widget.first_child();
    while let Some(widget) = child {
        capture_scroll(&widget, scroll);
        child = widget.next_sibling();
    }
}

#[cfg(test)]
mod tests;
