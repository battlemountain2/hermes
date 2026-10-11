use super::{anchored_offset, fit_zoom, scroll_zoom};

#[test]
fn fits_portrait_and_landscape_without_upscaling() {
    assert_eq!(fit_zoom(2000.0, 1000.0, 500.0, 600.0), 0.25);
    assert_eq!(fit_zoom(1000.0, 2000.0, 600.0, 500.0), 0.25);
    assert_eq!(fit_zoom(100.0, 100.0, 500.0, 600.0), 1.0);
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
    let preview = super::ImagePreview::new(&texture);
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
            std::thread::sleep(Duration::from_millis(10));
        }
    };
    pump(Duration::from_millis(500));
    let scroll = preview
        .widget
        .first_child()
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
    assert!((f64::from(area.content_width()) - scroll.hadjustment().page_size()).abs() < 2.0);
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
