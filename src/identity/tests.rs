use super::*;

#[test]
fn hermes_has_its_own_stable_application_and_development_identity() {
    assert_eq!(NAME, "Hermes");
    assert_eq!(application_id(false), APPLICATION_ID);
    assert_eq!(application_id(true), DEVELOPMENT_ID);
    assert!(gtk::gio::Application::id_is_valid(APPLICATION_ID));
    assert!(gtk::gio::Application::id_is_valid(DEVELOPMENT_ID));
    assert!(!APPLICATION_ID.contains("lgse"));
    assert!(!APPLICATION_ID.contains("Strata"));
    assert_eq!(ICON_NAME, APPLICATION_ID);
}

#[test]
fn desktop_launcher_and_resource_icon_match_the_application_identity() {
    let launcher = include_str!("../../packaging/linux/io.github.battlemountain2.Hermes.desktop");
    for field in [
        "Name=Hermes".to_owned(),
        "Exec=hermes %U".to_owned(),
        format!("Icon={ICON_NAME}"),
        format!("StartupWMClass={APPLICATION_ID}"),
    ] {
        assert!(launcher.lines().any(|line| line == field));
    }
    assert!(
        include_str!("../../data/strata.gresource.xml").contains(&format!("apps/{ICON_NAME}.svg"))
    );
    assert!(!launcher.contains("DBusActivatable=true"));
    let development =
        include_str!("../../packaging/linux/io.github.battlemountain2.Hermes.Dev.desktop");
    assert!(
        development
            .lines()
            .any(|line| line == format!("StartupWMClass={DEVELOPMENT_ID}"))
    );
    assert!(development.lines().any(|line| line == "NoDisplay=true"));
}

#[test]
#[ignore = "Private GTK display; validates embedded app icons without desktop installation"]
fn desktop_app_icon_is_available_at_small_sizes_and_symbolic() {
    gtk::init().unwrap();
    crate::assets::prepare().unwrap();
    crate::assets::register_icon_theme();
    let display = gtk::gdk::Display::default().unwrap();
    let icons = gtk::IconTheme::for_display(&display);
    assert!(icons.has_icon(ICON_NAME));
    assert!(icons.has_icon(&format!("{ICON_NAME}-symbolic")));
    for size in [16, 24, 32, 64] {
        let icon = icons.lookup_icon(
            ICON_NAME,
            &[],
            size,
            1,
            gtk::TextDirection::None,
            gtk::IconLookupFlags::empty(),
        );
        assert_eq!(
            icon.icon_name().as_deref(),
            Some(std::path::Path::new(ICON_NAME))
        );
        assert!(!icon.is_symbolic());
    }
    let icon = icons.lookup_icon(
        &format!("{ICON_NAME}-symbolic"),
        &[],
        16,
        1,
        gtk::TextDirection::None,
        gtk::IconLookupFlags::empty(),
    );
    assert!(icon.is_symbolic());
}
