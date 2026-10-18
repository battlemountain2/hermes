// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;
use std::path::PathBuf;

#[test]
fn test_draw_ambient_aura_renders_cleanly() {
    let surface = gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, 400, 300)
        .expect("create surface");
    let cr = gtk::cairo::Context::new(&surface).expect("create context");
    draw_ambient_aura(&cr, 400.0, 300.0, (0.85, 0.45, 0.2), (0.2, 0.6, 0.8));
}

#[test]
fn test_draw_generative_visualizers_all_styles_render_cleanly() {
    let surface = gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, 600, 400)
        .expect("create surface");
    let cr = gtk::cairo::Context::new(&surface).expect("create context");

    let spectrum = [0.5; crate::playback::SPECTRUM_BANDS];
    for style in [
        VisualizerStyle::Ribbons,
        VisualizerStyle::Tunnel,
        VisualizerStyle::Cosmic,
    ] {
        draw_generative_visualizer(
            &cr,
            600.0,
            400.0,
            1.25,
            style,
            &spectrum,
            (1.0, 0.42, 0.21),
            (0.0, 0.85, 1.0),
        );
    }
}

#[test]
#[ignore = "Requires isolated GTK display"]
fn test_immersive_view_widget_construction_headless() {
    gtk::init().expect("Isolated GTK display available");
    let controller = Controller::with_session(Some(PathBuf::from("/tmp/dummy-session.json")));
    let view = ImmersiveView::new(
        controller,
        Rc::new(|| {}),
        Rc::new(|| {}),
    );

    assert_eq!(view.state.visualizer_style.get(), VisualizerStyle::Ribbons);
    assert!(view.state.lyrics_active.get());
    assert!(!view.state.visualizer_active.get());
    assert!(view.widget().has_css_class("immersive-root"));
}
