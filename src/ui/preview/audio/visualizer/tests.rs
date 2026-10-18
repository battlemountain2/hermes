use super::*;

#[test]
fn peaks_hold_then_decay_to_idle_without_new_audio() {
    let mut peaks = [0.0; SPECTRUM_BANDS];
    let mut hold = [0; SPECTRUM_BANDS];
    update_peaks(&[1.0; SPECTRUM_BANDS], &mut peaks, &mut hold);
    assert_eq!(peaks, [1.0; SPECTRUM_BANDS]);
    for _ in 0..PEAK_HOLD_FRAMES {
        update_peaks(&[0.0; SPECTRUM_BANDS], &mut peaks, &mut hold);
        assert_eq!(peaks, [1.0; SPECTRUM_BANDS]);
    }
    update_peaks(&[0.0; SPECTRUM_BANDS], &mut peaks, &mut hold);
    assert_eq!(peaks, [PEAK_DECAY; SPECTRUM_BANDS]);
    for _ in 0..100 {
        update_peaks(&[0.0; SPECTRUM_BANDS], &mut peaks, &mut hold);
    }
    assert!(peaks.iter().all(|peak| *peak <= 0.02));
}

#[test]
#[ignore = "Private GTK display; tests weak stream/widget ownership without starting a decoder"]
fn desktop_visualizer_releases_stream_widgets_and_signal_handlers() {
    gtk::init().unwrap();
    for _ in 0..32 {
        let stream: PlaybackStream = glib::Object::new();
        let weak_stream = stream.downgrade();
        let visualizer = MicroSegmentVisualizer::new(&stream);
        let weak_widget = visualizer.widget().downgrade();
        assert!(visualizer.state.playing_handler.borrow().is_some());
        visualizer.stop();
        visualizer.stop();
        assert!(visualizer.state.playing_handler.borrow().is_none());
        drop(visualizer);
        assert!(weak_widget.upgrade().is_none());
        drop(stream);
        assert!(weak_stream.upgrade().is_none());
    }
    let stream: PlaybackStream = glib::Object::new();
    let weak_stream = stream.downgrade();
    let visualizer = MicroSegmentVisualizer::new(&stream);
    drop(stream);
    assert!(weak_stream.upgrade().is_none());
    drop(visualizer);
}

#[test]
#[ignore = "Private GTK display; tests mapped tick cancellation and shared reduced-motion policy without audio"]
fn desktop_visualizer_cancels_ticks_when_hidden_reduced_or_stopped() {
    gtk::init().unwrap();
    crate::ui::motion::set_reduce_motion(false);
    gtk::Settings::default()
        .unwrap()
        .set_gtk_enable_animations(true);
    let stream: PlaybackStream = glib::Object::new();
    let visualizer = MicroSegmentVisualizer::new(&stream);
    let window = gtk::Window::builder().child(visualizer.widget()).build();
    window.present();
    assert!(visualizer.widget().is_mapped());
    visualizer.state.peaks.set([1.0; SPECTRUM_BANDS]);
    visualizer.ensure_tick_running();
    assert!(visualizer.state.tick.borrow().is_some());
    let registered = format!("{:?}", visualizer.state.tick.borrow());
    visualizer.ensure_tick_running();
    assert_eq!(registered, format!("{:?}", visualizer.state.tick.borrow()));
    visualizer.widget().set_visible(false);
    assert!(visualizer.state.tick.borrow().is_none());
    assert_eq!(visualizer.state.peaks.get(), [0.0; SPECTRUM_BANDS]);
    visualizer.widget().set_visible(true);
    visualizer.state.peaks.set([1.0; SPECTRUM_BANDS]);
    visualizer.ensure_tick_running();
    assert!(visualizer.state.tick.borrow().is_some());
    crate::ui::motion::set_reduce_motion(true);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while visualizer.state.tick.borrow().is_some() {
        assert!(
            std::time::Instant::now() < deadline,
            "Reduced-motion tick did not stop"
        );
        glib::MainContext::default().iteration(false);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(visualizer.state.tick.borrow().is_none());
    assert_eq!(visualizer.state.bands.get(), [0.0; SPECTRUM_BANDS]);
    crate::ui::motion::set_reduce_motion(false);
    visualizer.state.peaks.set([1.0; SPECTRUM_BANDS]);
    visualizer.ensure_tick_running();
    assert!(visualizer.state.tick.borrow().is_some());
    gtk::Settings::default()
        .unwrap()
        .set_gtk_enable_animations(false);
    visualizer.ensure_tick_running();
    assert!(visualizer.state.tick.borrow().is_none());
    gtk::Settings::default()
        .unwrap()
        .set_gtk_enable_animations(true);
    visualizer.state.peaks.set([1.0; SPECTRUM_BANDS]);
    visualizer.ensure_tick_running();
    assert!(visualizer.state.tick.borrow().is_some());
    visualizer.stop();
    visualizer.ensure_tick_running();
    assert!(visualizer.state.tick.borrow().is_none());
    window.close();
    drop(visualizer);
    drop(stream);
}
