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
    let controller = Controller::with_session(None);
    let view = ImmersiveView::new(controller, Rc::new(|| {}), Rc::new(|| {}));

    assert_eq!(view.state.visualizer_style.get(), VisualizerStyle::Ribbons);
    assert!(view.state.lyrics_active.get());
    assert!(!view.state.visualizer_active.get());
    assert!(view.widget().has_css_class("immersive-root"));
    assert!(view.state.asset_request.borrow().is_none());
    assert!(view.state.asset_task.borrow().is_none());
    assert!(view.state.tick.borrow().is_none());
}

#[test]
fn asset_requests_reject_stale_identity_and_cancel_on_drop() {
    let previous = AssetRequest::new();
    let old_identity = previous.identity.clone();
    let cancellation = previous.cancellation.clone();
    assert!(previous.accepts(&old_identity));
    let current = AssetRequest::new();
    assert!(!current.accepts(&old_identity));
    drop(previous);
    assert!(cancellation.is_cancelled());
    assert!(current.accepts(&current.identity));
    current.cancellation.cancel();
    assert!(!current.accepts(&current.identity));
}

#[test]
fn visualizer_frames_require_visible_playing_motion_enabled_view() {
    for mask in 0..32 {
        let mapped = mask & 1 != 0;
        let stopped = mask & 2 != 0;
        let active = mask & 4 != 0;
        let playing = mask & 8 != 0;
        let motion = mask & 16 != 0;
        assert_eq!(
            needs_visualizer_frames(mapped, stopped, active, playing, motion),
            mask == 29,
        );
    }
}

#[test]
fn visualizer_caps_high_refresh_frames_and_resets_resume_time() {
    for refresh_rate in [60, 144, 240, 1000] {
        let mut previous = None;
        let mut deadline = None;
        let mut frames = 0;
        for frame in 0..refresh_rate {
            let now = frame * 1_000_000 / refresh_rate;
            if let Some((_, next)) = visualizer_frame_delta(previous, deadline, now) {
                previous = Some(now);
                deadline = Some(next);
                frames += 1;
            }
        }
        assert!(
            (59..=60).contains(&frames),
            "{refresh_rate}Hz generated {frames} frames"
        );
        if refresh_rate == 60 {
            assert_eq!(frames, 60, "Nominal 60Hz must not alternate or skip frames");
        }
    }
    assert_eq!(visualizer_frame_delta(Some(100), None, 200), None);
    assert_eq!(visualizer_frame_delta(Some(200), None, 100), None);
    let (delta, next) = visualizer_frame_delta(Some(100), None, 1_000_100).unwrap();
    assert_eq!(delta, 0.1);
    assert!(next > 1_000_100 && next <= 1_016_766);
    assert_eq!(
        visualizer_frame_delta(None, None, 1_000_100),
        Some((0.0, 1_016_766))
    );
}

#[test]
fn spectrum_motion_is_time_based_finite_and_has_a_gentler_release() {
    let initial = [0.0; 32];
    let target = [1.0; 32];
    let mut sixty = initial;
    for _ in 0..60 {
        sixty = smooth_spectrum(sixty, target, 1.0 / 60.0);
    }
    let mut high_refresh = initial;
    for _ in 0..144 {
        high_refresh = smooth_spectrum(high_refresh, target, 1.0 / 144.0);
    }
    assert!((sixty[0] - high_refresh[0]).abs() < 0.00001);
    let rise = smooth_spectrum(initial, target, 1.0 / 60.0)[0];
    let fall = smooth_spectrum(target, initial, 1.0 / 60.0)[0];
    assert!(rise > 0.0 && rise < 1.0);
    assert!(fall > 1.0 - rise && fall < 1.0);
    for seconds in [0.0, -1.0, f64::NAN, f64::INFINITY, 20.0] {
        let value = smooth_spectrum([f32::NAN; 32], [f32::INFINITY; 32], seconds);
        assert!(
            value
                .iter()
                .all(|band| band.is_finite() && (0.0..=1.0).contains(band))
        );
    }
}

#[test]
fn cancelled_asset_work_returns_without_loading_media() {
    let cancellation = Cancellation::default();
    cancellation.cancel();
    let assets = read_track_assets(Path::new("/unused/cancelled-track.flac"), &cancellation);
    assert!(assets.artwork.is_none());
    assert!(assets.lyrics.is_none());
}

fn fixture_view() -> ImmersiveView {
    ImmersiveView::new(
        Controller::with_session(None),
        Rc::new(|| {}),
        Rc::new(|| {}),
    )
}

#[test]
#[ignore = "Private GTK display/settings; synthetic lyrics and no audio or network"]
fn desktop_lyrics_and_visualizer_modes_switch_cleanly_and_keep_empty_layout_stable() {
    gtk::init().unwrap();
    assets::prepare().unwrap();
    let manager = ThemeManager::shared();
    manager.set_online_lyrics(false);
    let styles = gtk::CssProvider::new();
    styles.load_from_string(include_str!("../../../style.css"));
    gtk::style_context_add_provider_for_display(
        &gdk::Display::default().unwrap(),
        &styles,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
    let view = fixture_view();
    let window = gtk::Window::builder()
        .default_width(1200)
        .default_height(850)
        .child(view.widget())
        .build();
    window.present();
    assert_eq!(*view.state.lyrics_status.borrow(), LyricsStatus::NoTrack);
    manager.set_online_lyrics(true);
    assert_eq!(*view.state.lyrics_status.borrow(), LyricsStatus::NoTrack);
    manager.set_online_lyrics(false);
    view.state.lyrics_status.replace(LyricsStatus::LocalMissing);
    view.state.rebuild_lyrics_ui();
    assert!(view.state.lyrics_scroll.is_visible());
    let empty = view.state.lyrics_box.first_child().unwrap();
    for _ in 0..30 {
        view.sync();
        assert_eq!(view.state.lyrics_box.first_child().as_ref(), Some(&empty));
    }
    let lyrics = crate::services::lyrics::parse_lrc(
        "[00:00]Synthetic first line\n[00:02]Synthetic second line",
    )
    .unwrap();
    view.state.current_lyrics.replace(Some(lyrics));
    view.state.lyrics_status.replace(LyricsStatus::Local);
    view.state.rebuild_lyrics_ui();
    let labels = view.state.lyric_labels.borrow().clone();
    let size = |label: &gtk::Label| {
        let layout = label.create_pango_layout(Some("Synthetic stable line"));
        layout.pixel_size()
    };
    let original = size(&labels[0]);
    labels[0].add_css_class("active-lyric");
    assert_eq!(size(&labels[0]), original);
    labels[0].remove_css_class("active-lyric");
    let settle = || {
        let until = std::time::Instant::now() + Duration::from_millis(60);
        while std::time::Instant::now() < until {
            while glib::MainContext::default().iteration(false) {}
            std::thread::sleep(Duration::from_millis(2));
        }
    };
    settle();
    let original_bounds = (labels[0].width(), labels[0].height());
    labels[0].add_css_class("active-lyric");
    settle();
    assert_eq!((labels[0].width(), labels[0].height()), original_bounds);
    labels[0].remove_css_class("active-lyric");
    for _ in 0..5 {
        view.state.visualizer_btn.set_active(true);
        assert!(view.state.visualizer_active.get());
        assert!(!view.state.lyrics_btn.is_active());
        assert!(!view.state.stage_box.is_visible());
        view.state.lyrics_btn.set_active(true);
        assert!(!view.state.visualizer_active.get());
        assert!(!view.state.visualizer_btn.is_active());
        assert!(view.state.stage_box.is_visible());
        assert!(view.state.lyrics_scroll.is_visible());
        assert!(!view.state.visualizer_canvas.is_visible());
        assert!(view.state.tick.borrow().is_none());
        assert_eq!(*view.state.lyric_labels.borrow(), labels);
    }
    view.state.visualizer_btn.set_active(true);
    view.state.current_lyrics.replace(None);
    view.state.rebuild_lyrics_ui();
    view.state.lyrics_btn.set_active(true);
    assert!(view.state.lyrics_scroll.is_visible());
    assert!(view.state.lyrics_built.get());
    assert!(view.state.lyrics_box.first_child().is_some());
    view.request_stop();
    window.close();
}

#[test]
#[ignore = "Private GTK/preferences; injected provider results, no source files, decoder or network"]
fn desktop_online_lyrics_rejects_stale_results_cancels_on_opt_out_and_never_invents_timing() {
    gtk::init().unwrap();
    assets::prepare().unwrap();
    let manager = ThemeManager::shared();
    manager.set_online_lyrics(true);
    let view = fixture_view();
    let window = gtk::Window::builder().child(view.widget()).build();
    window.present();
    let track = fixture_track("/synthetic/lyrics.flac");
    arm_asset_request(&view, &track, 1);
    view.state.cancel_request();
    let old_request = AssetRequest::new();
    let old_identity = old_request.identity.clone();
    view.state.lyrics_request.replace(Some(old_request));
    let current_request = AssetRequest::new();
    let current_identity = current_request.identity.clone();
    let cancellation = current_request.cancellation.clone();
    view.state.lyrics_request.replace(Some(current_request));
    view.state
        .lyrics_status
        .replace(LyricsStatus::LoadingOnline);
    let result = || {
        Ok(online::Lookup::Found(
            crate::services::lyrics::parse_plain("Synthetic first line\nSynthetic second line")
                .unwrap(),
        ))
    };
    view.state
        .finish_online_lyrics(&old_identity, &track, 1, result());
    assert!(view.state.current_lyrics.borrow().is_none());
    view.state
        .finish_online_lyrics(&current_identity, &track, 2, result());
    assert!(view.state.current_lyrics.borrow().is_none());
    view.state
        .finish_online_lyrics(&current_identity, &track, 1, result());
    assert_eq!(*view.state.lyrics_status.borrow(), LyricsStatus::Online);
    assert!(!view.state.current_lyrics.borrow().as_ref().unwrap().synced);
    assert!(view.state.active_lyric_index.get().is_none());
    assert!(
        view.state
            .lyric_labels
            .borrow()
            .iter()
            .all(|label| label.is_selectable())
    );
    assert!(cancellation.is_cancelled());
    view.state.current_lyrics.replace(None);
    let hidden_request = AssetRequest::new();
    let hidden_identity = hidden_request.identity.clone();
    let hidden_cancelled = hidden_request.cancellation.clone();
    view.state.lyrics_request.replace(Some(hidden_request));
    view.state
        .lyrics_status
        .replace(LyricsStatus::LoadingOnline);
    view.state.visualizer_btn.set_active(true);
    assert!(hidden_cancelled.is_cancelled());
    assert!(view.state.lyrics_request.borrow().is_none());
    view.state
        .finish_online_lyrics(&hidden_identity, &track, 1, result());
    assert!(view.state.current_lyrics.borrow().is_none());
    view.state.lyrics_btn.set_active(true);
    let pending = AssetRequest::new();
    let late_identity = pending.identity.clone();
    let cancelled = pending.cancellation.clone();
    view.state.lyrics_request.replace(Some(pending));
    view.state
        .lyrics_status
        .replace(LyricsStatus::LoadingOnline);
    let task = glib::MainContext::default().spawn_local(std::future::pending::<()>());
    let source = task.source().clone();
    view.state.lyrics_task.replace(Some(task));
    manager.set_online_lyrics(false);
    assert!(cancelled.is_cancelled());
    assert!(source.is_destroyed());
    assert!(view.state.lyrics_request.borrow().is_none());
    view.state
        .finish_online_lyrics(&late_identity, &track, 1, result());
    assert!(view.state.current_lyrics.borrow().is_none());
    assert_eq!(
        *view.state.lyrics_status.borrow(),
        LyricsStatus::LocalMissing
    );
    view.request_stop();
    window.close();
}

#[test]
#[ignore = "Private GTK/preferences; verifies local-first gating with synthetic content, no network"]
fn desktop_local_lyrics_and_opt_out_prevent_online_requests() {
    gtk::init().unwrap();
    assets::prepare().unwrap();
    let manager = ThemeManager::shared();
    manager.set_online_lyrics(true);
    let view = fixture_view();
    let window = gtk::Window::builder().child(view.widget()).build();
    window.present();
    let mut track = fixture_track("/synthetic/local-first.flac");
    track.tags.title = Some("Synthetic title".into());
    track.tags.artist = Some("Synthetic artist".into());
    arm_asset_request(&view, &track, 1);
    view.state.cancel_request();
    let stream: crate::ui::preview::stream::PlaybackStream = glib::Object::new();
    stream.stream_prepared(true, false, true, 180_000_000);
    view.state.controller.stream.replace(Some(stream));
    view.state
        .current_lyrics
        .replace(crate::services::lyrics::parse_lrc(
            "[00:00]Local synthetic line",
        ));
    view.state.lyrics_status.replace(LyricsStatus::Local);
    view.state.maybe_lookup_lyrics();
    assert!(view.state.lyrics_request.borrow().is_none());
    assert!(!view.state.lyrics_attempted.get());
    manager.set_online_lyrics(false);
    view.state.current_lyrics.replace(None);
    view.state.lyrics_status.replace(LyricsStatus::LocalMissing);
    view.state.maybe_lookup_lyrics();
    assert!(view.state.lyrics_request.borrow().is_none());
    assert!(!view.state.lyrics_attempted.get());
    view.request_stop();
    window.close();
}

fn fixture_track(name: &str) -> Track {
    Track {
        path: PathBuf::from(name),
        tags: Default::default(),
    }
}

fn arm_asset_request(view: &ImmersiveView, track: &Track, generation: u64) -> Rc<()> {
    view.state.cancel_request();
    view.state
        .controller
        .queue
        .borrow_mut()
        .replace(vec![track.clone()], 0);
    view.state.controller.generation.set(generation);
    view.state.current_track.replace(Some(track.clone()));
    view.state.current_generation.set(generation);
    view.state.track_dirty.set(false);
    let request = AssetRequest::new();
    let identity = request.identity.clone();
    view.state.asset_request.replace(Some(request));
    identity
}

fn fixture_assets(text: &str) -> TrackAssets {
    let surface = gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, 2, 2)
        .expect("artwork fixture");
    let mut bytes = Vec::new();
    surface.write_to_png(&mut bytes).expect("artwork bytes");
    TrackAssets {
        artwork: Some(bytes),
        lyrics: crate::services::lyrics::parse_lrc(&format!("[00:00]{text}")),
    }
}

#[test]
#[ignore = "Requires isolated GTK display; run alone; synthetic artwork and no playback"]
fn desktop_immersive_rejects_stale_artwork_success_and_failure() {
    gtk::init().expect("isolated GTK display");
    let view = fixture_view();
    let window = gtk::Window::builder().child(view.widget()).build();
    window.present();
    assert!(view.widget().is_mapped());
    let old_track = fixture_track("old.flac");
    let old_identity = arm_asset_request(&view, &old_track, 1);
    let cancellation = view
        .state
        .asset_request
        .borrow()
        .as_ref()
        .unwrap()
        .cancellation
        .clone();
    let track = fixture_track("new.flac");
    let identity = arm_asset_request(&view, &track, 2);
    assert!(cancellation.is_cancelled());
    view.state
        .finish_track_assets(&old_identity, &old_track, 1, Some(fixture_assets("stale")));
    assert!(view.state.current_lyrics.borrow().is_none());
    assert!(view.state.art_picture.paintable().is_none());
    view.state
        .finish_track_assets(&identity, &track, 2, Some(fixture_assets("current")));
    let paintable = view.state.art_picture.paintable().expect("current artwork");
    assert_eq!(
        view.state.current_lyrics.borrow().as_ref().unwrap().lines[0].text,
        "current"
    );
    assert_eq!(
        view.state.art_stack.visible_child_name().as_deref(),
        Some("picture")
    );
    view.state
        .finish_track_assets(&old_identity, &old_track, 1, None);
    assert_eq!(view.state.art_picture.paintable(), Some(paintable.clone()));
    assert_eq!(
        view.state.art_stack.visible_child_name().as_deref(),
        Some("picture")
    );
    let revisited_identity = arm_asset_request(&view, &old_track, 3);
    view.state.finish_track_assets(
        &old_identity,
        &old_track,
        1,
        Some(fixture_assets("stale revisit")),
    );
    assert_eq!(view.state.art_picture.paintable(), Some(paintable));
    view.state.finish_track_assets(
        &revisited_identity,
        &old_track,
        3,
        Some(fixture_assets("revisited")),
    );
    assert_eq!(
        view.state.current_lyrics.borrow().as_ref().unwrap().lines[0].text,
        "revisited"
    );
    view.request_stop();
    window.close();
}

#[test]
#[ignore = "Requires isolated GTK display; run alone; no files, decoder, or audio output"]
fn desktop_immersive_defers_hidden_work_and_cleans_up_requests_and_drop() {
    gtk::init().expect("isolated GTK display");
    let view = fixture_view();
    let track = fixture_track("never-opened.flac");
    view.state
        .controller
        .queue
        .borrow_mut()
        .replace(vec![track.clone()], 0);
    view.sync();
    assert!(view.state.current_track.borrow().is_none());
    assert!(view.state.asset_request.borrow().is_none());
    assert!(view.state.asset_task.borrow().is_none());
    assert!(view.state.lyric_labels.borrow().is_empty());
    assert_eq!(view.state.title_label.text(), "Nothing playing");
    view.state
        .controller
        .queue
        .borrow_mut()
        .replace(Vec::new(), 0);
    let window = gtk::Window::builder().child(view.widget()).build();
    window.present();
    let identity = arm_asset_request(&view, &track, 1);
    let cancellation = view
        .state
        .asset_request
        .borrow()
        .as_ref()
        .unwrap()
        .cancellation
        .clone();
    let task = glib::MainContext::default().spawn_local(std::future::pending::<()>());
    let source = task.source().clone();
    view.state.asset_task.replace(Some(task));
    view.widget().set_visible(false);
    assert!(cancellation.is_cancelled());
    assert!(source.is_destroyed());
    assert!(view.state.asset_request.borrow().is_none());
    assert!(view.state.asset_task.borrow().is_none());
    view.state
        .finish_track_assets(&identity, &track, 1, Some(fixture_assets("hidden")));
    assert!(view.state.current_lyrics.borrow().is_none());
    view.state
        .controller
        .queue
        .borrow_mut()
        .replace(Vec::new(), 0);
    view.widget().set_visible(true);
    assert!(!view.state.track_dirty.get());
    let request = AssetRequest::new();
    let cancellation = request.cancellation.clone();
    view.state.asset_request.replace(Some(request));
    let weak_state = Rc::downgrade(&view.state);
    let retained_widget = view.widget().clone();
    let clone = view.clone();
    drop(view);
    assert!(!cancellation.is_cancelled());
    drop(clone);
    assert!(cancellation.is_cancelled());
    assert!(weak_state.upgrade().is_none());
    assert!(retained_widget.is_mapped());
    window.close();
}

#[test]
#[ignore = "Requires isolated GTK display; run alone; synthetic unprepared stream without media"]
fn desktop_immersive_tick_motion_pause_stop_and_filled_play_icons() {
    gtk::init().expect("isolated GTK display");
    gio::resources_register_include!("strata.gresource").expect("bundled icons");
    assets::register_icon_theme();
    let _theme_manager = ThemeManager::shared();
    crate::ui::motion::set_reduce_motion(false);
    let settings = gtk::Settings::default().expect("GTK settings");
    let previous_animations = settings.is_gtk_enable_animations();
    settings.set_gtk_enable_animations(true);
    let view = fixture_view();
    let pixels = |image: &gtk::Image| {
        assert!(!assets::has_primary_icon(image));
        let texture = image
            .paintable()
            .expect("icon paintable")
            .downcast::<gdk::Texture>()
            .expect("icon texture");
        let stride = texture.width() as usize * 4;
        let mut bytes = vec![0; stride * texture.height() as usize];
        texture.download(&mut bytes, stride);
        bytes
    };
    let assert_icons = |name: &str| {
        for (button, size) in [
            (&view.state.play_button, 20),
            (&view.state.floating_play, 16),
        ] {
            let image = button
                .child()
                .expect("play image")
                .downcast::<gtk::Image>()
                .unwrap();
            assert_eq!(pixels(&image), pixels(&assets::on_accent_icon(name, size)));
        }
    };
    assert_icons(icons::PLAY);
    let stream: crate::ui::preview::stream::PlaybackStream = glib::Object::new();
    view.state.controller.stream.replace(Some(stream));
    view.state.controller.play_intent.set(true);
    let window = gtk::Window::builder().child(view.widget()).build();
    window.present();
    assert_icons(icons::PAUSE);
    assert!(view.state.tick.borrow().is_none());
    view.state.visualizer_btn.set_active(true);
    assert!(view.state.tick.borrow().is_some());
    view.state.last_frame_us.set(Some(100));
    view.widget().set_visible(false);
    assert!(view.state.tick.borrow().is_none());
    assert_eq!(view.state.last_frame_us.get(), None);
    view.widget().set_visible(true);
    assert!(view.state.tick.borrow().is_some());
    assert_eq!(view.state.last_frame_us.get(), None);
    crate::ui::motion::set_reduce_motion(true);
    view.sync();
    assert!(view.state.tick.borrow().is_none());
    crate::ui::motion::set_reduce_motion(false);
    view.sync();
    assert!(view.state.tick.borrow().is_some());
    settings.set_gtk_enable_animations(false);
    view.sync();
    assert!(view.state.tick.borrow().is_none());
    settings.set_gtk_enable_animations(true);
    view.sync();
    assert!(view.state.tick.borrow().is_some());
    view.state.controller.play_intent.set(false);
    view.sync();
    assert!(view.state.tick.borrow().is_none());
    assert_icons(icons::PLAY);
    view.state.controller.play_intent.set(true);
    view.sync();
    assert_eq!(view.state.last_frame_us.get(), None);
    let request = AssetRequest::new();
    let cancellation = request.cancellation.clone();
    view.state.asset_request.replace(Some(request));
    let task = glib::MainContext::default().spawn_local(std::future::pending::<()>());
    let source = task.source().clone();
    view.state.asset_task.replace(Some(task));
    view.request_stop();
    view.request_stop();
    view.sync();
    assert!(view.state.tick.borrow().is_none());
    assert!(view.state._observer.borrow().is_none());
    assert!(cancellation.is_cancelled());
    assert!(source.is_destroyed());
    assert!(view.state.asset_request.borrow().is_none());
    assert!(view.state.asset_task.borrow().is_none());
    window.close();
    settings.set_gtk_enable_animations(previous_animations);
}
