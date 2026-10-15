use super::{anchored_offset, fit_zoom, scroll_zoom};

#[test]
fn repaint_clears_old_image_pixels_and_keeps_drawing_inside_the_viewport() {
    let pixels =
        gtk::gdk_pixbuf::Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, true, 8, 40, 40).unwrap();
    pixels.fill(0xff0000ff);
    let mut surface =
        gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, 100, 100).unwrap();
    let context = gtk::cairo::Context::new(&surface).unwrap();
    let red = super::ImagePixels::new(pixels.clone());
    assert!(red.surface.is_some());
    let background = gtk::gdk::RGBA::new(0.0, 0.0, 0.0, 1.0);
    super::paint_image(&context, &red, 3.0, (100.0, 100.0), (0.0, 0.0), &background);
    pixels.fill(0x0000ffff);
    let blue = super::ImagePixels::new(pixels.clone());
    super::paint_image(
        &context,
        &blue,
        1.0,
        (100.0, 100.0),
        (0.0, 0.0),
        &background,
    );
    drop(context);
    let stride = surface.stride() as usize;
    let data = surface.data().unwrap();
    assert_eq!(
        &data[..4],
        &[0, 0, 0, 255],
        "Old zoomed pixels must disappear"
    );
    let center = 50 * stride + 50 * 4;
    assert_eq!(
        u32::from_ne_bytes(data[center..center + 4].try_into().unwrap()),
        0xff0000ff
    );
}

#[test]
fn zoom_easing_is_monotonic_and_converges_without_overshoot() {
    let mut scale = 1.0;
    for _ in 0..45 {
        let next = super::smooth_zoom(scale, 2.0, 1.0 / 60.0);
        assert!(next >= scale && next <= 2.0);
        scale = next;
    }
    assert_eq!(scale, 2.0);
    assert_eq!(super::smooth_zoom(2.0, 1.0, 0.0), 2.0);
}

#[test]
fn fits_portrait_and_landscape_without_upscaling() {
    assert_eq!(fit_zoom(2000.0, 1000.0, 500.0, 600.0), 0.25);
    assert_eq!(fit_zoom(1000.0, 2000.0, 600.0, 500.0), 0.25);
    assert_eq!(fit_zoom(100.0, 100.0, 500.0, 600.0), 1.0);
}

#[test]
fn thumbnail_and_full_image_start_with_the_same_fitted_geometry() {
    let low = super::initial_fit(256.0, 128.0, 600.0, 800.0, true);
    let full = super::initial_fit(1400.0, 700.0, 600.0, 800.0, false);
    assert_eq!((256.0 * low, 128.0 * low), (1400.0 * full, 700.0 * full));
    assert_eq!(super::initial_fit(100.0, 50.0, 600.0, 800.0, false), 1.0);
}

#[test]
#[ignore = "Private GTK display; generated GSK/Cairo frames only"]
fn desktop_image_letterbox_covers_the_previous_frame_instead_of_ghosting() {
    use gtk::prelude::*;
    gtk::init().unwrap();
    let pixels =
        gtk::gdk_pixbuf::Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, true, 8, 400, 200).unwrap();
    pixels.fill(0x2050e0ff);
    let texture = gtk::gdk::Texture::for_pixbuf(&pixels);
    let preview = super::ImagePreview::new(&texture, None);
    let window = gtk::Window::builder()
        .child(&preview.widget)
        .default_width(500)
        .default_height(500)
        .build();
    let styles = gtk::CssProvider::new();
    styles.load_from_string(
        "@define-color theme_bg rgba(12,22,32,0.4); .hermes-image-canvas { color: @theme_bg; }",
    );
    let display = gtk::prelude::WidgetExt::display(&window);
    gtk::style_context_add_provider_for_display(
        &display,
        &styles,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    window.present();
    let viewport = preview.widget.first_child().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(250);
    while std::time::Instant::now() < deadline {
        for _ in 0..64 {
            if !gtk::glib::MainContext::default().iteration(false) {
                break;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(viewport.width() > 400 && viewport.height() > 300);
    let snapshot = gtk::Snapshot::new();
    gtk::WidgetPaintable::new(Some(&viewport)).snapshot(
        &snapshot,
        f64::from(viewport.width()),
        f64::from(viewport.height()),
    );
    let node = snapshot.to_node().unwrap();
    let mut surface = gtk::cairo::ImageSurface::create(
        gtk::cairo::Format::ARgb32,
        viewport.width(),
        viewport.height(),
    )
    .unwrap();
    let drawing = gtk::cairo::Context::new(&surface).unwrap();
    drawing.set_source_rgb(1.0, 0.0, 0.0);
    drawing.paint().unwrap();
    node.draw(&drawing);
    drop(drawing);
    let data = surface.data().unwrap();
    assert_eq!(
        &data[..4],
        &[32, 22, 12, 255],
        "The old red frame must not show through letterboxing"
    );
    drop(data);
    preview.stop();
    window.close();
    gtk::style_context_remove_provider_for_display(&display, &styles);
}

#[test]
fn zoom_keeps_centered_image_point_under_pointer() {
    assert_eq!(anchored_offset(0.0, 250.0, 500.0, 300.0, 600.0), 50.0);
    assert_eq!(anchored_offset(0.0, 200.0, 500.0, 300.0, 900.0), 100.0);
    assert_eq!(anchored_offset(100.0, 200.0, 500.0, 900.0, 300.0), 0.0);
    assert_eq!(anchored_offset(100.0, 200.0, 500.0, 900.0, 1800.0), 400.0);
}

#[test]
fn zoom_does_not_jump_from_tiny_fit_and_rejects_invalid_scroll() {
    assert!(scroll_zoom(0.005, -1.0) < 0.01);
    assert!(scroll_zoom(0.005, 1.0) < 0.005);
    assert_eq!(scroll_zoom(0.000001, 100.0), 0.000001);
    assert_eq!(scroll_zoom(10.0, -100.0), 10.0);
    assert_eq!(scroll_zoom(1.0, f64::NAN), 1.0);
}

#[test]
#[ignore = "Requires a desktop display; run explicitly"]
fn desktop_image_zoom_and_fullscreen() {
    use gtk::prelude::*;
    use std::time::{Duration, Instant};

    gtk::init().expect("Desktop display available");
    crate::assets::prepare().expect("Bundled icons load");
    let pixels =
        gtk::gdk_pixbuf::Pixbuf::new(gtk::gdk_pixbuf::Colorspace::Rgb, true, 8, 2000, 1000)
            .unwrap();
    pixels.fill(0x90aabbff);
    let texture = gtk::gdk::Texture::for_pixbuf(&pixels);
    let preview = super::ImagePreview::new(&texture, None);
    let window = gtk::Window::builder()
        .default_width(500)
        .default_height(400)
        .child(&preview.widget)
        .build();
    window.present();
    let pump = |duration: Duration| {
        let until = Instant::now() + duration;
        while Instant::now() < until {
            while gtk::glib::MainContext::default().iteration(false) {}
            if let Some(clock) = window.frame_clock() {
                clock.emit_by_name::<()>("update", &[]);
                clock.emit_by_name::<()>("layout", &[]);
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    };
    pump(Duration::from_millis(500));
    let scroll = preview
        .widget
        .first_child()
        .unwrap()
        .downcast::<gtk::Overlay>()
        .unwrap()
        .child()
        .unwrap()
        .downcast::<gtk::ScrolledWindow>()
        .unwrap();
    let controls = preview.widget.last_child().unwrap();
    let fit = controls
        .first_child()
        .unwrap()
        .downcast::<gtk::Button>()
        .unwrap();
    let actual = fit
        .next_sibling()
        .unwrap()
        .downcast::<gtk::Button>()
        .unwrap();
    let area = scroll
        .child()
        .unwrap()
        .downcast::<gtk::Viewport>()
        .unwrap()
        .child()
        .unwrap()
        .downcast::<gtk::DrawingArea>()
        .unwrap();
    assert!(scroll.hadjustment().upper() <= scroll.hadjustment().page_size() + 1.0);
    assert!(scroll.vadjustment().upper() <= scroll.vadjustment().page_size() + 1.0);
    window.set_default_size(700, 450);
    pump(Duration::from_millis(400));
    assert!(
        (f64::from(area.content_width()) - scroll.hadjustment().page_size()).abs() < 2.0,
        "Fit content {}x{}, viewport {}x{}, scroll {}x{}, window {}x{}",
        area.content_width(),
        area.content_height(),
        scroll.hadjustment().page_size(),
        scroll.vadjustment().page_size(),
        scroll.width(),
        scroll.height(),
        window.width(),
        window.height()
    );
    assert!(scroll.vadjustment().upper() <= scroll.vadjustment().page_size() + 1.0);
    actual.emit_clicked();
    pump(Duration::from_millis(300));
    assert_eq!(area.content_width(), 2000);
    assert_eq!(area.content_height(), 1000);
    assert!(
        (scroll.hadjustment().value() - (2000.0 - scroll.hadjustment().page_size()) / 2.0).abs()
            < 2.0
    );
    for position in 0..scroll.observe_controllers().n_items() {
        if let Some(motion) = scroll
            .observe_controllers()
            .item(position)
            .and_then(|controller| controller.downcast::<gtk::EventControllerMotion>().ok())
            .filter(|controller| controller.name().as_deref() == Some("hermes-image-pointer"))
        {
            motion.emit_by_name::<()>(
                "motion",
                &[
                    &(scroll.hadjustment().page_size() / 2.0),
                    &(scroll.vadjustment().page_size() / 2.0),
                ],
            );
        }
    }
    for position in 0..scroll.observe_controllers().n_items() {
        if let Some(wheel) = scroll
            .observe_controllers()
            .item(position)
            .and_then(|controller| controller.downcast::<gtk::EventControllerScroll>().ok())
            .filter(|controller| controller.name().as_deref() == Some("hermes-image-zoom"))
        {
            wheel.emit_by_name::<bool>("scroll", &[&0.0_f64, &-1.0_f64]);
        }
    }
    pump(Duration::from_millis(300));
    assert_eq!(area.content_width(), 2300);
    assert!(
        (scroll.hadjustment().value() - (2300.0 - scroll.hadjustment().page_size()) / 2.0).abs()
            < 2.0,
        "Offset {} page {} upper {}",
        scroll.hadjustment().value(),
        scroll.hadjustment().page_size(),
        scroll.hadjustment().upper()
    );
    let wheel = (0..scroll.observe_controllers().n_items())
        .find_map(|position| {
            scroll
                .observe_controllers()
                .item(position)
                .and_then(|controller| controller.downcast::<gtk::EventControllerScroll>().ok())
                .filter(|controller| controller.name().as_deref() == Some("hermes-image-zoom"))
        })
        .unwrap();
    wheel.emit_by_name::<bool>("scroll", &[&0.0_f64, &-1.0_f64]);
    wheel.emit_by_name::<bool>("scroll", &[&0.0_f64, &-1.0_f64]);
    pump(Duration::from_millis(300));
    let extent = 2000.0 * 1.15_f64.powi(3);
    assert!(
        (scroll.hadjustment().value() - (extent - scroll.hadjustment().page_size()) / 2.0).abs()
            < 2.0
    );
    let viewport = preview
        .widget
        .first_child()
        .unwrap()
        .downcast::<gtk::Overlay>()
        .unwrap();
    let canvas = viewport.last_child().unwrap();
    assert!(canvas.width() <= scroll.width());
    assert!(canvas.height() <= scroll.height());
    assert!(
        area.content_width() > canvas.width() * 3,
        "Only the empty extent, not the image canvas, grows when zooming"
    );
    let drag = (0..scroll.observe_controllers().n_items())
        .find_map(|position| {
            scroll
                .observe_controllers()
                .item(position)
                .and_downcast::<gtk::GestureDrag>()
        })
        .unwrap();
    let start_x = scroll.hadjustment().value();
    drag.emit_by_name::<()>("drag-begin", &[&0.0_f64, &0.0_f64]);
    drag.emit_by_name::<()>("drag-update", &[&50.0_f64, &30.0_f64]);
    pump(Duration::from_millis(200));
    assert!((scroll.hadjustment().value() - (start_x - 50.0)).abs() < 2.0);
    for _ in 0..6 {
        let before_zoom = scroll.hadjustment().value();
        let previous_extent = f64::from(area.content_width());
        wheel.emit_by_name::<bool>("scroll", &[&0.0_f64, &-0.4_f64]);
        pump(Duration::from_millis(35));
        let before_drag = anchored_offset(
            before_zoom,
            scroll.hadjustment().page_size() / 2.0,
            scroll.hadjustment().page_size(),
            previous_extent,
            f64::from(area.content_width()),
        );
        drag.emit_by_name::<()>("drag-begin", &[&0.0_f64, &0.0_f64]);
        drag.emit_by_name::<()>("drag-update", &[&10.0_f64, &10.0_f64]);
        pump(Duration::from_millis(100));
        assert!(
            (scroll.hadjustment().value() - (before_drag - 10.0)).abs() < 3.0,
            "Zoom-to-pan transition jumped: expected {}, actual {}",
            before_drag - 10.0,
            scroll.hadjustment().value()
        );
        assert!(canvas.width() <= scroll.width());
        assert!(canvas.height() <= scroll.height());
    }
    fit.emit_clicked();
    pump(Duration::from_millis(300));
    assert!(scroll.hadjustment().upper() <= scroll.hadjustment().page_size() + 1.0);
    assert!(preview.open_fullscreen());
    pump(Duration::from_millis(300));
    let fullscreen = preview.fullscreen.borrow().as_ref().unwrap().clone();
    for position in 0..fullscreen.observe_controllers().n_items() {
        if let Some(keys) = fullscreen
            .observe_controllers()
            .item(position)
            .and_then(|controller| controller.downcast::<gtk::EventControllerKey>().ok())
        {
            keys.emit_by_name::<bool>(
                "key-pressed",
                &[&gtk::gdk::Key::f, &0_u32, &gtk::gdk::ModifierType::empty()],
            );
        }
    }
    pump(Duration::from_millis(100));
    assert!(preview.fullscreen.borrow().is_none());
    preview.open_fullscreen();
    pump(Duration::from_millis(100));
    preview.stop();
    assert!(preview.fullscreen.borrow().is_none());
    window.close();
    pump(Duration::from_millis(100));
}
