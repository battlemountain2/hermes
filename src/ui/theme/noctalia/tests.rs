use std::{
    fs,
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};

use super::*;

pub(crate) const CSS: &str = r##"
/* Noctalia GTK4 export: surface, surface_container, on_surface, primary, error */
@define-color window_bg_color #101417;
@define-color card_bg_color #1c2024;
@define-color window_fg_color #dfe3e7;
@define-color accent_bg_color #94cdf6;
@define-color destructive_bg_color #ffb4ab;
@define-color headerbar_backdrop_color @window_bg_color;
@define-color theme_selected_bg_color @accent_bg_color;
:root { --accent-color: #94cdf6; }
"##;

pub(crate) struct Fixture(pub std::path::PathBuf);

impl Fixture {
    pub fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let directory = std::env::temp_dir().join(format!(
            "hermes-noctalia-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&directory).expect("create palette fixture");
        Self(directory)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).expect("remove palette fixture");
    }
}

#[test]
fn installed_v5_gtk_export_maps_to_semantic_tokens() {
    let tokens = tokens_from_css(CSS).expect("valid Noctalia GTK export");
    assert_eq!(tokens.name, "Noctalia");
    assert_eq!(tokens.background, "#101417");
    assert_eq!(tokens.surface, "#1c2024");
    assert_eq!(tokens.text, "#dfe3e7");
    assert_eq!(tokens.accent, "#94cdf6");
    assert_eq!(tokens.danger, "#ffb4ab");
    assert_eq!(tokens.muted, blend("#1c2024", "#dfe3e7", 0.08));
    assert_eq!(tokens.highlight, blend("#1c2024", "#94cdf6", 0.18));
    assert_eq!(tokens.border, blend("#1c2024", "#dfe3e7", 0.24));
    assert_eq!(tokens.dim_text, blend("#1c2024", "#dfe3e7", 0.65));
}

#[test]
fn light_export_uses_light_surfaces_without_dark_assumptions() {
    let source = CSS
        .replace("#101417", "#f7fafc")
        .replace("#1c2024", "#eef1f4")
        .replace("#dfe3e7", "#14171a");
    let tokens = tokens_from_css(&source).expect("light export");
    assert_eq!(tokens.background, "#f7fafc");
    assert_eq!(tokens.surface, "#eef1f4");
    assert_eq!(tokens.text, "#14171a");
}

#[test]
fn missing_required_roles_reject_the_entire_update() {
    for role in [
        "window_bg_color",
        "card_bg_color",
        "window_fg_color",
        "accent_bg_color",
        "destructive_bg_color",
    ] {
        let source = CSS
            .lines()
            .filter(|line| !line.starts_with(&format!("@define-color {role} ")))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(tokens_from_css(&source).is_none(), "missing {role}");
    }
}

#[test]
fn aliases_are_resolved_without_loading_external_css() {
    let source = CSS.replace("window_fg_color #dfe3e7", "window_fg_color @foreground")
        + "@define-color foreground #DFE3E7;";
    assert_eq!(
        tokens_from_css(&source).expect("alias export").text,
        "#dfe3e7"
    );
}

#[test]
fn comments_do_not_supply_or_override_colors() {
    let source = format!("/* @define-color window_bg_color #ffffff; */{CSS}");
    assert_eq!(
        tokens_from_css(&source)
            .expect("comment ignored")
            .background,
        "#101417"
    );
    assert!(tokens_from_css(&format!("{CSS}/* unfinished")).is_none());
}

#[test]
fn malformed_colors_css_injection_transparency_and_alias_cycles_are_rejected() {
    for invalid in [
        "#zzzzzz",
        "#12345",
        "#ffffff00",
        "transparent",
        "red",
        "rgb(1,2,3)",
        "@missing",
        "@accent_bg_color",
        "#123456 extra",
        "#123456\n@import url(evil)",
    ] {
        assert!(
            tokens_from_css(&CSS.replace("#94cdf6", invalid)).is_none(),
            "accepted {invalid}"
        );
    }
    assert!(
        tokens_from_css(
            &(CSS.to_owned() + "@define-color first @second; @define-color second @first;")
        )
        .is_none()
    );
}

#[test]
fn duplicate_and_incomplete_declarations_are_rejected() {
    for suffix in [
        "@define-color window_bg_color #ffffff;",
        "@define-color broken",
        "@define-color broken ;",
        "@define-color-broken #123456;",
    ] {
        assert!(tokens_from_css(&(CSS.to_owned() + suffix)).is_none());
    }
}

#[test]
fn short_hex_is_normalized_and_output_never_contains_expressions() {
    let tokens = tokens_from_css(&CSS.replace("#94cdf6", "#ABC")).expect("short hex");
    assert_eq!(tokens.accent, "#aabbcc");
}

#[test]
fn oversized_exports_are_bounded() {
    assert!(tokens_from_css(&(CSS.to_owned() + &" ".repeat(MAX_PALETTE_BYTES))).is_none());
    let fixture = Fixture::new();
    let path = fixture.0.join("noctalia.css");
    fs::write(&path, CSS.to_owned() + &" ".repeat(MAX_PALETTE_BYTES))
        .expect("write oversized export");
    assert!(read_palette(&path).is_none());
}

#[test]
fn too_many_definitions_bound_alias_resolution_work() {
    let mut source = CSS.to_owned();
    for index in 0..128 {
        source.push_str(&format!("@define-color extra{index} #123456;"));
    }
    assert!(tokens_from_css(&source).is_none());
}

#[test]
fn invalid_utf8_and_directories_are_not_palettes() {
    let fixture = Fixture::new();
    let path = fixture.0.join("noctalia.css");
    fs::write(&path, [0xff, 0xfe]).expect("write malformed export");
    assert!(read_palette(&path).is_none());
    assert!(read_palette(&fixture.0).is_none());
}

#[test]
fn last_valid_survives_partial_writes_removal_and_atomic_replacement() {
    let fixture = Fixture::new();
    let path = fixture.0.join("noctalia.css");
    let mut palette = Palette::new(path.clone());
    assert!(!palette.refresh());
    assert!(palette.last_valid.is_none());
    fs::write(&path, CSS).expect("write export");
    assert!(palette.refresh());
    let last = palette.last_valid.clone();
    assert!(!palette.refresh());
    fs::write(&path, "@define-color window_bg_color #112233;").expect("partial write");
    assert!(!palette.refresh());
    assert_eq!(palette.last_valid, last);
    fs::remove_file(&path).expect("remove export");
    assert!(!palette.refresh());
    assert_eq!(palette.last_valid, last);
    let temporary = fixture.0.join("palette.tmp");
    fs::write(&temporary, CSS.replace("#94cdf6", "#abcdef")).expect("replacement");
    fs::rename(&temporary, &path).expect("atomic replace");
    assert!(palette.refresh());
    assert_eq!(palette.last_valid.expect("new palette").accent, "#abcdef");
}

#[test]
fn export_path_honors_the_supplied_xdg_config_root() {
    assert_eq!(
        palette_path(Path::new("/custom/config")),
        Path::new("/custom/config/gtk-4.0/noctalia.css")
    );
}

#[test]
fn directory_events_include_atomic_rename_targets_and_directory_recreation() {
    let path = Path::new("/config/gtk-4.0/noctalia.css");
    assert!(relevant_event(path, path));
    assert!(relevant_event(path, Path::new("/config/gtk-4.0")));
    assert!(!relevant_event(
        path,
        Path::new("/config/gtk-4.0/noctalia.css.tmp")
    ));
    assert!(!relevant_event(path, Path::new("/config/gtk-4.0/gtk.css")));
}

#[test]
fn watching_moves_between_existing_ancestors_as_directories_appear_or_disappear() {
    let fixture = Fixture::new();
    let path = palette_path(&fixture.0);
    assert_eq!(watch_directory(&path), Some(fixture.0.clone()));
    let gtk = fixture.0.join("gtk-4.0");
    fs::create_dir(&gtk).expect("create export directory");
    assert_eq!(watch_directory(&path), Some(gtk.clone()));
    fs::remove_dir(&gtk).expect("remove export directory");
    assert_eq!(watch_directory(&path), Some(fixture.0.clone()));
}
