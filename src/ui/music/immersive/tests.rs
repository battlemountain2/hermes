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
    let view = ImmersiveView::new(
        controller,
        Rc::new(|| {}),
        Rc::new(|| {}),
    );

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
        let mut frames = 0;
        for frame in 0..refresh_rate {
            let now = frame * 1_000_000 / refresh_rate;
            if visualizer_frame_delta(previous, now).is_some() {
                previous = Some(now);
                frames += 1;
            }
        }
        assert!(frames <= 60, "{refresh_rate}Hz generated {frames} frames");
        if refresh_rate == 60 {
            assert_eq!(frames, 60, "Nominal 60Hz must not alternate or skip frames");
        }
    }
    assert_eq!(visualizer_frame_delta(Some(100), 200), None);
    assert_eq!(visualizer_frame_delta(Some(200), 100), None);
    assert_eq!(visualizer_frame_delta(Some(100), 1_000_100), Some(0.1));
    assert_eq!(visualizer_frame_delta(None, 1_000_100), Some(0.0));
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
    ImmersiveView::new(Controller::with_session(None), Rc::new(|| {}), Rc::new(|| {}))
}

fn fixture_track(name: &str) -> Track {
    Track {
        path: PathBuf::from(name),
        tags: Default::default(),
    }
}

fn arm_asset_request(view: &ImmersiveView, track: &Track, generation: u64) -> Rc<()> {
    view.state.cancel_request();
    view.state.controller.queue.borrow_mut().replace(vec![track.clone()], 0);
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
    let cancellation = view.state.asset_request.borrow().as_ref().unwrap().cancellation.clone();
    let track = fixture_track("new.flac");
    let identity = arm_asset_request(&view, &track, 2);
    assert!(cancellation.is_cancelled());
    view.state.finish_track_assets(&old_identity, &old_track, 1, Some(fixture_assets("stale")));
    assert!(view.state.current_lyrics.borrow().is_none());
    assert!(view.state.art_picture.paintable().is_none());
    view.state.finish_track_assets(&identity, &track, 2, Some(fixture_assets("current")));
    let paintable = view.state.art_picture.paintable().expect("current artwork");
    assert_eq!(view.state.current_lyrics.borrow().as_ref().unwrap().lines[0].text, "current");
    assert_eq!(view.state.art_stack.visible_child_name().as_deref(), Some("picture"));
    view.state.finish_track_assets(&old_identity, &old_track, 1, None);
    assert_eq!(view.state.art_picture.paintable(), Some(paintable.clone()));
    assert_eq!(view.state.art_stack.visible_child_name().as_deref(), Some("picture"));
    let revisited_identity = arm_asset_request(&view, &old_track, 3);
    view.state.finish_track_assets(&old_identity, &old_track, 1, Some(fixture_assets("stale revisit")));
    assert_eq!(view.state.art_picture.paintable(), Some(paintable));
    view.state.finish_track_assets(&revisited_identity, &old_track, 3, Some(fixture_assets("revisited")));
    assert_eq!(view.state.current_lyrics.borrow().as_ref().unwrap().lines[0].text, "revisited");
    view.request_stop();
    window.close();
}

#[test]
#[ignore = "Requires isolated GTK display; run alone; no files, decoder, or audio output"]
fn desktop_immersive_defers_hidden_work_and_cleans_up_requests_and_drop() {
    gtk::init().expect("isolated GTK display");
    let view = fixture_view();
    let track = fixture_track("never-opened.flac");
    view.state.controller.queue.borrow_mut().replace(vec![track.clone()], 0);
    view.sync();
    assert!(view.state.current_track.borrow().is_none());
    assert!(view.state.asset_request.borrow().is_none());
    assert!(view.state.asset_task.borrow().is_none());
    assert!(view.state.lyric_labels.borrow().is_empty());
    assert_eq!(view.state.title_label.text(), "Nothing playing");
    view.state.controller.queue.borrow_mut().replace(Vec::new(), 0);
    let window = gtk::Window::builder().child(view.widget()).build();
    window.present();
    let identity = arm_asset_request(&view, &track, 1);
    let cancellation = view.state.asset_request.borrow().as_ref().unwrap().cancellation.clone();
    let task = glib::MainContext::default().spawn_local(std::future::pending::<()>());
    let source = task.source().clone();
    view.state.asset_task.replace(Some(task));
    view.widget().set_visible(false);
    assert!(cancellation.is_cancelled());
    assert!(source.is_destroyed());
    assert!(view.state.asset_request.borrow().is_none());
    assert!(view.state.asset_task.borrow().is_none());
    view.state.finish_track_assets(&identity, &track, 1, Some(fixture_assets("hidden")));
    assert!(view.state.current_lyrics.borrow().is_none());
    view.state.controller.queue.borrow_mut().replace(Vec::new(), 0);
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
        let texture = image.paintable().expect("icon paintable")
            .downcast::<gdk::Texture>().expect("icon texture");
        let stride = texture.width() as usize * 4;
        let mut bytes = vec![0; stride * texture.height() as usize];
        texture.download(&mut bytes, stride);
        bytes
    };
    let assert_icons = |name: &str| {
        for (button, size) in [(&view.state.play_button, 20), (&view.state.floating_play, 16)] {
            let image = button.child().expect("play image").downcast::<gtk::Image>().unwrap();
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
