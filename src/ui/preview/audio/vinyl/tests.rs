use super::*;

use std::time::Duration;

#[test]
fn rotation_accelerates_toward_original_33_rpm() {
    let mut rotation = Rotation::default();
    let elapsed = 1.0 / 60.0;
    assert!((TARGET_ANGULAR_VELOCITY - 3.4906585).abs() < 0.000001);
    assert!(rotation.advance(true, elapsed));
    assert!((rotation.velocity - TARGET_ANGULAR_VELOCITY * elapsed * 6.0).abs() < 1e-12);
    assert!((rotation.angle - rotation.velocity * elapsed).abs() < 1e-12);

    for _ in 0..300 {
        let previous = rotation.velocity;
        assert!(rotation.advance(true, elapsed));
        assert!(rotation.velocity >= previous);
        assert!(rotation.velocity <= TARGET_ANGULAR_VELOCITY);
    }
    assert!((rotation.velocity - TARGET_ANGULAR_VELOCITY).abs() < 1e-12);
}

#[test]
fn rotation_pause_uses_original_deceleration_and_settles() {
    let mut rotation = Rotation {
        angle: 1.0,
        velocity: TARGET_ANGULAR_VELOCITY,
    };
    let elapsed = 1.0 / 60.0;
    assert!(rotation.advance(false, elapsed));
    assert!((rotation.velocity - TARGET_ANGULAR_VELOCITY * (1.0 - elapsed * 4.5)).abs() < 1e-12);
    assert!(rotation.angle > 1.0);

    let mut settled = false;
    for _ in 0..300 {
        if !rotation.advance(false, elapsed) {
            settled = true;
            break;
        }
    }
    assert!(settled);
    assert_eq!(rotation.velocity, 0.0);
    let resting_angle = rotation.angle;
    assert!(!rotation.advance(false, elapsed));
    assert_eq!(rotation.angle, resting_angle);
}

#[test]
fn rotation_wraps_angle_without_changing_speed() {
    let mut rotation = Rotation {
        angle: 2.0 * PI - 0.01,
        velocity: TARGET_ANGULAR_VELOCITY,
    };
    assert!(rotation.advance(true, 0.016));
    assert!((rotation.angle - (TARGET_ANGULAR_VELOCITY * 0.016 - 0.01)).abs() < 1e-12);
    assert_eq!(rotation.velocity, TARGET_ANGULAR_VELOCITY);
}

#[test]
fn rotation_caps_elapsed_after_stall() {
    for playing in [false, true] {
        let mut capped = Rotation {
            angle: 0.7,
            velocity: 1.2,
        };
        let mut stalled = capped;
        assert_eq!(capped.advance(playing, 0.1), stalled.advance(playing, 12.0));
        assert_eq!(capped.velocity, stalled.velocity);
        assert_eq!(capped.angle, stalled.angle);
    }
}

#[test]
fn animation_state_gates_visibility_motion_and_disposal() {
    let animation = VinylAnimation::default();
    assert!(!animation.should_animate(true, true));
    animation.playing.set(true);
    assert!(animation.should_animate(true, true));
    assert!(!animation.should_animate(false, true));
    assert!(!animation.should_animate(true, false));

    animation.playing.set(false);
    animation.rotation.set(Rotation {
        angle: 0.5,
        velocity: 1.0,
    });
    assert!(animation.should_animate(true, true));
    assert!(!animation.should_animate(false, true));
    assert!(!animation.should_animate(true, false));

    animation.stopped.set(true);
    animation.playing.set(true);
    assert!(!animation.should_animate(true, true));
}

#[test]
fn animation_cancellation_preserves_angle_and_play_intent() {
    let animation = VinylAnimation::default();
    animation.rotation.set(Rotation {
        angle: 2.5,
        velocity: TARGET_ANGULAR_VELOCITY,
    });
    animation.playing.set(true);
    animation.last_tick.set(Some(Instant::now()));

    animation.cancel_tick();
    animation.cancel_tick();
    assert_eq!(animation.rotation.get().angle, 2.5);
    assert_eq!(animation.rotation.get().velocity, 0.0);
    assert_eq!(animation.last_tick.get(), None);
    assert!(animation.playing.get());
    assert!(animation.should_animate(true, true));
}

struct GtkFixture {
    window: gtk::Window,
    settings: gtk::Settings,
    original_animations: bool,
    original_options: audio_preferences::Options,
    _theme_manager: Rc<ThemeManager>,
}

impl GtkFixture {
    fn new() -> Self {
        assert_eq!(
            std::env::var("HERMES_VINYL_TEST_ISOLATED").as_deref(),
            Ok("1"),
            "Run one exact ignored test per process with private HOME/XDG, D-Bus and GTK display"
        );
        gtk::init().expect("Private GTK display available");
        let theme_manager = ThemeManager::shared();
        let settings = gtk::Settings::default().unwrap();
        let original_animations = settings.is_gtk_enable_animations();
        let original_options = audio_preferences::current();
        audio_preferences::configure(true, 32, "studio");
        settings.set_gtk_enable_animations(true);
        crate::ui::motion::set_reduce_motion(false);
        Self {
            window: gtk::Window::new(),
            settings,
            original_animations,
            original_options,
            _theme_manager: theme_manager,
        }
    }

    fn mount(&self, disc: &VinylDisc) {
        self.window.set_child(Some(disc.widget()));
        self.window.present();
        wait_until(|| disc.widget().is_mapped());
    }
}

impl Drop for GtkFixture {
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

fn wait_until(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(Instant::now() < deadline, "GTK condition timed out");
        for _ in 0..32 {
            if !glib::MainContext::default().iteration(false) {
                break;
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn assert_motion_stopped(disc: &VinylDisc) {
    assert!(disc.animation.tick.borrow().is_none());
    assert_eq!(disc.animation.rotation.get().velocity, 0.0);
    assert_eq!(disc.animation.last_tick.get(), None);
}

#[test]
#[ignore = "Private GTK/HOME/XDG/D-Bus; run one exact test per process, no audio or decoding"]
fn gtk_vinyl_unmap_remap_and_refresh_keep_one_tick() {
    let fixture = GtkFixture::new();
    let disc = VinylDisc::new();
    let inactive_weak_count = Rc::weak_count(&disc.animation);
    disc.set_playing(true);
    assert_motion_stopped(&disc);
    fixture.mount(&disc);
    wait_until(|| disc.animation.rotation.get().velocity > 0.0);
    assert_eq!(Rc::weak_count(&disc.animation), inactive_weak_count + 1);

    let started_at = disc.animation.last_tick.get();
    let tick = disc.animation.tick.borrow();
    assert!(tick.is_some());
    for _ in 0..20 {
        disc.set_playing(true);
    }
    assert_eq!(disc.animation.last_tick.get(), started_at);
    assert_eq!(Rc::weak_count(&disc.animation), inactive_weak_count + 1);
    drop(tick);

    for _ in 0..3 {
        disc.widget().set_visible(false);
        assert!(!disc.widget().is_mapped());
        assert_motion_stopped(&disc);
        assert_eq!(Rc::weak_count(&disc.animation), inactive_weak_count);
        let hidden_angle = disc.animation.rotation.get().angle;
        disc.set_playing(true);
        assert_motion_stopped(&disc);
        assert_eq!(disc.animation.rotation.get().angle, hidden_angle);
        disc.widget().set_visible(true);
        wait_until(|| disc.animation.tick.borrow().is_some());
        wait_until(|| disc.animation.rotation.get().velocity > 0.0);
        assert_eq!(Rc::weak_count(&disc.animation), inactive_weak_count + 1);
    }
}

#[test]
#[ignore = "Private GTK/HOME/XDG/D-Bus; run one exact test per process, no audio or decoding"]
fn gtk_vinyl_pause_eases_to_rest() {
    let fixture = GtkFixture::new();
    let disc = VinylDisc::new();
    disc.set_playing(true);
    fixture.mount(&disc);
    wait_until(|| disc.animation.rotation.get().velocity > 1.0);
    let velocity = disc.animation.rotation.get().velocity;
    let angle = disc.animation.rotation.get().angle;
    disc.set_playing(false);
    disc.set_playing(false);
    assert!(disc.animation.tick.borrow().is_some());
    assert_eq!(disc.animation.rotation.get().velocity, velocity);
    wait_until(|| disc.animation.rotation.get().velocity < velocity);
    assert_ne!(disc.animation.rotation.get().angle, angle);
    wait_until(|| disc.animation.tick.borrow().is_none());
    assert_motion_stopped(&disc);
    disc.set_playing(false);
    assert_motion_stopped(&disc);
    disc.set_playing(true);
    assert!(disc.animation.tick.borrow().is_some());
}

#[test]
#[ignore = "Private GTK/HOME/XDG/D-Bus; run one exact test per process, no audio or decoding"]
fn gtk_vinyl_stop_and_drop_release_callbacks() {
    let fixture = GtkFixture::new();
    let disc = VinylDisc::new();
    let inactive_weak_count = Rc::weak_count(&disc.animation);
    disc.set_playing(true);
    fixture.mount(&disc);
    wait_until(|| disc.animation.rotation.get().velocity > 0.0);
    let angle = disc.animation.rotation.get().angle;
    disc.stop();
    disc.stop();
    assert_motion_stopped(&disc);
    assert!(disc.animation.preferences_observer.borrow().is_none());
    assert_eq!(Rc::weak_count(&disc.animation), inactive_weak_count - 1);
    disc.set_playing(true);
    disc.widget().set_visible(false);
    disc.widget().set_visible(true);
    assert_motion_stopped(&disc);
    assert_eq!(disc.animation.rotation.get().angle, angle);
    assert!(!disc.animation.playing.get());

    let weak_animation = Rc::downgrade(&disc.animation);
    let widget = disc.widget().clone();
    drop(disc);
    assert!(weak_animation.upgrade().is_none());
    let weak_widget = widget.downgrade();
    fixture.window.set_child(gtk::Widget::NONE);
    drop(widget);
    assert!(weak_widget.upgrade().is_none());

    let disc = VinylDisc::new();
    let inactive_weak_count = Rc::weak_count(&disc.animation);
    disc.set_playing(true);
    fixture.mount(&disc);
    wait_until(|| disc.animation.tick.borrow().is_some());
    let animation = disc.animation.clone();
    let widget = disc.widget().clone();
    drop(disc);
    assert!(widget.is_mapped());
    assert!(animation.tick.borrow().is_none());
    assert!(animation.preferences_observer.borrow().is_none());
    assert_eq!(Rc::weak_count(&animation), inactive_weak_count - 1);
    assert_eq!(animation.rotation.get().velocity, 0.0);
    let weak_animation = Rc::downgrade(&animation);
    drop(animation);
    assert!(weak_animation.upgrade().is_none());
}

fn assert_motion_policy_refresh(fixture: &GtkFixture, set_enabled: impl Fn(bool)) {
    let disc = VinylDisc::new();
    set_enabled(false);
    disc.set_playing(true);
    fixture.mount(&disc);
    disc.set_playing(true);
    assert_motion_stopped(&disc);

    set_enabled(true);
    assert_motion_stopped(&disc);
    disc.set_playing(true);
    wait_until(|| disc.animation.rotation.get().velocity > 0.0);

    set_enabled(false);
    wait_until(|| disc.animation.tick.borrow().is_none());
    assert_motion_stopped(&disc);
    assert!(disc.animation.playing.get());
    let angle = disc.animation.rotation.get().angle;
    disc.set_playing(true);
    assert_motion_stopped(&disc);
    assert_eq!(disc.animation.rotation.get().angle, angle);

    set_enabled(true);
    assert_motion_stopped(&disc);
    disc.set_playing(true);
    assert!(disc.animation.tick.borrow().is_some());
    set_enabled(false);
    disc.set_playing(true);
    assert_motion_stopped(&disc);

    set_enabled(true);
    disc.set_playing(true);
    wait_until(|| disc.animation.rotation.get().velocity > 0.1);
    disc.set_playing(false);
    assert!(disc.animation.tick.borrow().is_some());
    set_enabled(false);
    wait_until(|| disc.animation.tick.borrow().is_none());
    assert_motion_stopped(&disc);
}

#[test]
#[ignore = "Private GTK/HOME/XDG/D-Bus; run one exact test per process, no audio or decoding"]
fn gtk_vinyl_reduce_motion_stops_active_ticks_and_refresh_resumes() {
    let fixture = GtkFixture::new();
    assert_motion_policy_refresh(&fixture, |enabled| {
        crate::ui::motion::set_reduce_motion(!enabled);
    });
}

#[test]
#[ignore = "Private GTK/HOME/XDG/D-Bus; run one exact test per process, no audio or decoding"]
fn gtk_vinyl_gtk_animation_setting_stops_active_ticks_and_refresh_resumes() {
    let fixture = GtkFixture::new();
    assert_motion_policy_refresh(&fixture, |enabled| {
        fixture.settings.set_gtk_enable_animations(enabled);
    });
}

#[test]
#[ignore = "Private GTK/HOME/XDG/D-Bus; run one exact test per process, no audio or decoding"]
fn gtk_vinyl_live_spin_options_preserve_intent_and_lifecycle() {
    let fixture = GtkFixture::new();
    audio_preferences::configure(false, 16, "smooth");
    let disc = VinylDisc::new();
    disc.set_playing(true);
    fixture.mount(&disc);
    assert_motion_stopped(&disc);
    assert!(disc.animation.playing.get());

    audio_preferences::configure(true, 16, "smooth");
    wait_until(|| disc.animation.rotation.get().velocity > 0.0);
    let angle = disc.animation.rotation.get().angle;
    audio_preferences::configure(false, 32, "snappy");
    assert_motion_stopped(&disc);
    assert_eq!(disc.animation.rotation.get().angle, angle);
    assert!(disc.animation.playing.get());
    audio_preferences::configure(true, 32, "studio");
    assert!(disc.animation.tick.borrow().is_some());
    let started_at = disc.animation.last_tick.get();
    audio_preferences::configure(true, 16, "smooth");
    assert_eq!(disc.animation.last_tick.get(), started_at);

    audio_preferences::configure(false, 16, "smooth");
    disc.set_playing(false);
    audio_preferences::configure(true, 16, "smooth");
    assert_motion_stopped(&disc);
    assert_eq!(disc.animation.rotation.get().angle, angle);
    disc.set_playing(true);
    assert!(disc.animation.tick.borrow().is_some());

    disc.widget().set_visible(false);
    audio_preferences::configure(false, 32, "studio");
    audio_preferences::configure(true, 32, "studio");
    assert_motion_stopped(&disc);
    disc.widget().set_visible(true);
    assert!(disc.animation.tick.borrow().is_some());

    crate::ui::motion::set_reduce_motion(true);
    audio_preferences::configure(false, 16, "snappy");
    audio_preferences::configure(true, 16, "snappy");
    assert_motion_stopped(&disc);
    crate::ui::motion::set_reduce_motion(false);
    fixture.settings.set_gtk_enable_animations(false);
    audio_preferences::configure(false, 32, "smooth");
    audio_preferences::configure(true, 32, "smooth");
    assert_motion_stopped(&disc);
    fixture.settings.set_gtk_enable_animations(true);
    audio_preferences::configure(true, 16, "studio");
    assert!(disc.animation.tick.borrow().is_some());
}

#[test]
#[ignore = "Private GTK/HOME/XDG/D-Bus; run one exact test per process, no audio or decoding"]
fn gtk_vinyl_live_options_observer_releases_on_stop_and_drop() {
    let fixture = GtkFixture::new();
    for stop_first in [true, false] {
        let disc = VinylDisc::new();
        disc.set_playing(true);
        fixture.mount(&disc);
        let observer = Rc::downgrade(
            disc.animation
                .preferences_observer
                .borrow()
                .as_ref()
                .unwrap(),
        );
        let animation = disc.animation.clone();
        let widget = disc.widget().clone();
        if stop_first {
            disc.stop();
            assert!(observer.upgrade().is_none());
            audio_preferences::configure(false, 16, "snappy");
            audio_preferences::configure(true, 32, "studio");
            assert_motion_stopped(&disc);
        }
        drop(disc);
        assert!(observer.upgrade().is_none());
        assert!(animation.tick.borrow().is_none());
        assert!(!animation.playing.get());
        let weak_animation = Rc::downgrade(&animation);
        drop(animation);
        assert!(weak_animation.upgrade().is_none());
        audio_preferences::configure(false, 16, "smooth");
        audio_preferences::configure(true, 32, "studio");
        let weak_widget = widget.downgrade();
        fixture.window.set_child(gtk::Widget::NONE);
        drop(widget);
        assert!(weak_widget.upgrade().is_none());
    }
}
