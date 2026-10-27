use super::*;

#[test]
fn peaks_hold_then_decay_to_idle_without_new_audio() {
    for (decay, expected_hold, expected_factor) in [
        (Decay::Studio, 10, 0.92),
        (Decay::Snappy, 4, 0.80),
        (Decay::Smooth, 18, 0.96),
    ] {
        let mut peaks = [0.0; SPECTRUM_BANDS];
        let mut hold = [0; SPECTRUM_BANDS];
        update_peaks(&[1.0; SPECTRUM_BANDS], &mut peaks, &mut hold, decay);
        assert_eq!(peaks, [1.0; SPECTRUM_BANDS]);
        assert_eq!(hold, [expected_hold; SPECTRUM_BANDS]);
        for remaining in (0..expected_hold).rev() {
            update_peaks(&[0.0; SPECTRUM_BANDS], &mut peaks, &mut hold, decay);
            assert_eq!(peaks, [1.0; SPECTRUM_BANDS]);
            assert_eq!(hold, [remaining; SPECTRUM_BANDS]);
        }
        update_peaks(&[0.0; SPECTRUM_BANDS], &mut peaks, &mut hold, decay);
        assert_eq!(peaks, [expected_factor; SPECTRUM_BANDS]);
        for _ in 0..100 {
            update_peaks(&[0.0; SPECTRUM_BANDS], &mut peaks, &mut hold, decay);
        }
        assert!(peaks.iter().all(|peak| *peak <= 0.02));
    }
}

#[test]
fn sixteen_columns_group_adjacent_bands_and_peaks_by_independent_maxima() {
    assert_eq!(SPECTRUM_BANDS, 32);
    let bands = std::array::from_fn(|band| band as f32 / 32.0);
    let peaks = std::array::from_fn(|band| 1.0 - band as f32 / 32.0);
    let grouped_bands = group_columns(&bands, 16);
    let grouped_peaks = group_columns(&peaks, 16);
    for column in 0..16 {
        assert_eq!(grouped_bands[column], bands[column * 2 + 1]);
        assert_eq!(grouped_peaks[column], peaks[column * 2]);
    }
    assert_eq!(grouped_bands[16..], [0.0; 16]);
    assert_eq!(grouped_peaks[16..], [0.0; 16]);
    assert_eq!(bands[31], 31.0 / 32.0);
    assert_eq!(peaks[0], 1.0);
}

#[test]
fn thirty_two_columns_preserve_every_analyzer_band_and_peak() {
    let bands = std::array::from_fn(|band| band as f32 / 32.0);
    let peaks = std::array::from_fn(|band| 1.0 - band as f32 / 32.0);
    assert_eq!(group_columns(&bands, 32), bands);
    assert_eq!(group_columns(&peaks, 32), peaks);
}

#[test]
fn decay_profile_changes_clamp_existing_hold_without_extending_it() {
    let mut hold = std::array::from_fn(|band| (band % 19) as u8);
    let original = hold;
    clamp_peak_hold(&mut hold, Decay::Snappy);
    assert_eq!(hold, original.map(|remaining| remaining.min(4)));
    let clamped = hold;
    clamp_peak_hold(&mut hold, Decay::Smooth);
    assert_eq!(hold, clamped);
    clamp_peak_hold(&mut hold, Decay::Studio);
    assert_eq!(hold, clamped);

    let mut peaks = [1.0; SPECTRUM_BANDS];
    for _ in 0..4 {
        update_peaks(&[0.0; SPECTRUM_BANDS], &mut peaks, &mut hold, Decay::Snappy);
    }
    assert_eq!(hold, [0; SPECTRUM_BANDS]);
    update_peaks(&[0.0; SPECTRUM_BANDS], &mut peaks, &mut hold, Decay::Snappy);
    assert!(peaks.iter().all(|peak| *peak <= 0.80));
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
    let _theme_manager = ThemeManager::shared();
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

struct PreferencesFixture {
    window: gtk::Window,
    settings: gtk::Settings,
    original_animations: bool,
    original_options: Options,
    _theme_manager: Rc<ThemeManager>,
}

impl PreferencesFixture {
    fn new() -> Self {
        assert_eq!(
            std::env::var("HERMES_AUDIO_PREFERENCES_TEST_ISOLATED").as_deref(),
            Ok("1"),
            "Run one exact ignored test per process with private HOME/XDG, D-Bus and GTK display"
        );
        gtk::init().expect("Private GTK display available");
        let theme_manager = ThemeManager::shared();
        let settings = gtk::Settings::default().unwrap();
        let original_animations = settings.is_gtk_enable_animations();
        let original_options = audio_preferences::current();
        settings.set_gtk_enable_animations(true);
        crate::ui::motion::set_reduce_motion(false);
        audio_preferences::configure(true, 32, "smooth");
        Self {
            window: gtk::Window::new(),
            settings,
            original_animations,
            original_options,
            _theme_manager: theme_manager,
        }
    }

    fn mount(&self, visualizer: &MicroSegmentVisualizer) {
        self.window.set_child(Some(visualizer.widget()));
        self.window.present();
        assert!(visualizer.widget().is_mapped());
    }
}

impl Drop for PreferencesFixture {
    fn drop(&mut self) {
        self.window.destroy();
        self.settings
            .set_gtk_enable_animations(self.original_animations);
        crate::ui::motion::set_reduce_motion(false);
        audio_preferences::configure(
            self.original_options.spin,
            self.original_options.columns as u32,
            self.original_options.decay.id(),
        );
    }
}

#[test]
#[ignore = "Private GTK/HOME/XDG/D-Bus; run one exact test per process, no audio or decoding"]
fn gtk_visualizer_live_options_update_columns_decay_and_lifecycle() {
    let fixture = PreferencesFixture::new();
    let stream: PlaybackStream = glib::Object::new();
    let visualizer = MicroSegmentVisualizer::new(&stream);
    assert_eq!(visualizer.state.options.get(), audio_preferences::current());
    fixture.mount(&visualizer);
    assert!(!stream.is_playing());
    assert!(visualizer.state.tick.borrow().is_none());
    let bands = std::array::from_fn(|band| band as f32 / 32.0);
    visualizer.state.bands.set(bands);
    visualizer.state.peaks.set([1.0; SPECTRUM_BANDS]);
    visualizer.state.peak_hold.set([18; SPECTRUM_BANDS]);

    audio_preferences::configure(true, 16, "snappy");
    assert_eq!(visualizer.state.options.get().columns, 16);
    assert_eq!(visualizer.state.options.get().decay, Decay::Snappy);
    assert_eq!(visualizer.state.bands.get(), bands);
    assert_eq!(visualizer.state.peaks.get(), [1.0; SPECTRUM_BANDS]);
    assert_eq!(visualizer.state.peak_hold.get(), [4; SPECTRUM_BANDS]);
    assert!(visualizer.state.tick.borrow().is_some());
    let registered = format!("{:?}", visualizer.state.tick.borrow());
    audio_preferences::configure(false, 32, "studio");
    assert_eq!(visualizer.state.options.get().columns, 32);
    assert_eq!(registered, format!("{:?}", visualizer.state.tick.borrow()));
    assert_eq!(visualizer.state.peak_hold.get(), [4; SPECTRUM_BANDS]);

    audio_preferences::configure(true, 16, "snappy");
    visualizer.state.cancel_tick();
    for remaining in (0..4).rev() {
        assert_eq!(
            visualizer.state.frame(visualizer.widget()),
            glib::ControlFlow::Continue
        );
        assert_eq!(visualizer.state.peaks.get(), [1.0; SPECTRUM_BANDS]);
        assert_eq!(
            visualizer.state.peak_hold.get(),
            [remaining; SPECTRUM_BANDS]
        );
    }
    assert_eq!(
        visualizer.state.frame(visualizer.widget()),
        glib::ControlFlow::Continue
    );
    assert_eq!(visualizer.state.peaks.get(), [0.80; SPECTRUM_BANDS]);
    assert_eq!(visualizer.state.bands.get(), [0.0; SPECTRUM_BANDS]);
    audio_preferences::configure(true, 32, "smooth");
    assert_eq!(visualizer.state.peak_hold.get(), [0; SPECTRUM_BANDS]);
    visualizer.state.cancel_tick();
    assert_eq!(
        visualizer.state.frame(visualizer.widget()),
        glib::ControlFlow::Continue
    );
    assert_eq!(visualizer.state.peaks.get(), [0.80 * 0.96; SPECTRUM_BANDS]);
    visualizer.ensure_tick_running();
    assert!(visualizer.state.tick.borrow().is_some());

    visualizer.widget().set_visible(false);
    audio_preferences::configure(true, 16, "snappy");
    assert_eq!(visualizer.state.options.get().columns, 16);
    assert!(visualizer.state.tick.borrow().is_none());
    assert_eq!(visualizer.state.peaks.get(), [0.0; SPECTRUM_BANDS]);
    visualizer.widget().set_visible(true);
    assert!(visualizer.state.tick.borrow().is_none());

    crate::ui::motion::set_reduce_motion(true);
    visualizer.state.peaks.set([1.0; SPECTRUM_BANDS]);
    audio_preferences::configure(true, 32, "smooth");
    assert!(visualizer.state.tick.borrow().is_none());
    assert_eq!(visualizer.state.peaks.get(), [0.0; SPECTRUM_BANDS]);
    crate::ui::motion::set_reduce_motion(false);
    fixture.settings.set_gtk_enable_animations(false);
    visualizer.state.peaks.set([1.0; SPECTRUM_BANDS]);
    audio_preferences::configure(true, 16, "snappy");
    assert!(visualizer.state.tick.borrow().is_none());
    assert_eq!(visualizer.state.peaks.get(), [0.0; SPECTRUM_BANDS]);
    fixture.settings.set_gtk_enable_animations(true);
    audio_preferences::configure(true, 32, "studio");
    assert!(visualizer.state.tick.borrow().is_none());

    visualizer.state.peaks.set([1.0; SPECTRUM_BANDS]);
    audio_preferences::configure(true, 16, "snappy");
    visualizer.state.cancel_tick();
    for _ in 0..100 {
        if visualizer.state.frame(visualizer.widget()) == glib::ControlFlow::Break {
            break;
        }
    }
    assert_eq!(visualizer.state.peaks.get(), [0.0; SPECTRUM_BANDS]);
    audio_preferences::configure(true, 32, "smooth");
    assert!(visualizer.state.tick.borrow().is_none());
}

#[test]
#[ignore = "Private GTK/HOME/XDG/D-Bus; run one exact test per process, no audio or decoding"]
fn gtk_visualizer_live_options_observer_releases_on_stop_and_drop() {
    let fixture = PreferencesFixture::new();
    for stop_first in [true, false] {
        let stream: PlaybackStream = glib::Object::new();
        let weak_stream = stream.downgrade();
        let visualizer = MicroSegmentVisualizer::new(&stream);
        fixture.mount(&visualizer);
        visualizer.state.peaks.set([1.0; SPECTRUM_BANDS]);
        visualizer.ensure_tick_running();
        assert!(visualizer.state.tick.borrow().is_some());
        let observer = Rc::downgrade(
            visualizer
                .state
                .preferences_observer
                .borrow()
                .as_ref()
                .unwrap(),
        );
        let state = visualizer.state.clone();
        let widget = visualizer.widget().clone();
        if stop_first {
            visualizer.stop();
            visualizer.stop();
            assert!(observer.upgrade().is_none());
            let options = state.options.get();
            audio_preferences::configure(false, 16, "snappy");
            assert_eq!(state.options.get(), options);
            assert!(state.tick.borrow().is_none());
            assert_eq!(state.peaks.get(), [0.0; SPECTRUM_BANDS]);
        }
        drop(visualizer);
        assert!(observer.upgrade().is_none());
        assert!(state.tick.borrow().is_none());
        assert!(state.playing_handler.borrow().is_none());
        let weak_state = Rc::downgrade(&state);
        drop(state);
        assert!(weak_state.upgrade().is_none());
        audio_preferences::configure(true, 32, "smooth");
        let weak_widget = widget.downgrade();
        fixture.window.set_child(gtk::Widget::NONE);
        drop(widget);
        assert!(weak_widget.upgrade().is_none());
        drop(stream);
        assert!(weak_stream.upgrade().is_none());
    }
}
