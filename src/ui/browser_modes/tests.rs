use super::*;

pub(in crate::ui) type LifecycleProbes = Vec<(String, glib::WeakRef<glib::Object>)>;

pub(in crate::ui) fn append_widget_probes(
    widget: &gtk::Widget,
    presentation: &str,
    probes: &mut LifecycleProbes,
) {
    probes.push((
        format!("{presentation} {}", widget.type_().name()),
        widget.upcast_ref::<glib::Object>().downgrade(),
    ));
    let mut child = widget.first_child();
    while let Some(widget) = child {
        append_widget_probes(&widget, presentation, probes);
        child = widget.next_sibling();
    }
}

pub(in crate::ui) fn append_pane_probes(views: &ModeViews, probes: &mut LifecycleProbes) {
    for (presentation, pane) in views
        .grid_panes
        .iter()
        .map(|pane| ("grid", pane))
        .chain(views.explorer_pane.iter().map(|pane| ("explorer", pane)))
    {
        append_widget_probes(pane.shell.upcast_ref(), presentation, probes);
        for object in [
            pane.model.upcast_ref::<glib::Object>(),
            pane.selection.upcast_ref::<glib::Object>(),
        ] {
            probes.push((
                format!("{presentation} {}", object.type_().name()),
                object.downgrade(),
            ));
        }
        if let Some(model) = &pane.filtered_model {
            probes.push((
                format!("{presentation} filtered model"),
                model.upcast_ref::<glib::Object>().downgrade(),
            ));
        }
        for bound in pane.bound_items.borrow().iter() {
            if let Some(item) = bound.item.upgrade() {
                probes.push((
                    format!("{presentation} item"),
                    item.upcast_ref::<glib::Object>().downgrade(),
                ));
            }
        }
    }
}
