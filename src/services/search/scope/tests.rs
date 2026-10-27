use super::*;

fn scoped() -> ScopedSearch {
    ScopedSearch::new(
        SearchScope::ThisFolder,
        Path::new("/synthetic/music"),
        "artist:\"A & B\"",
    )
}

#[test]
fn physical_groups_and_music_entity_context_round_trip_without_fake_locations() {
    for group in [SearchGroup::All, SearchGroup::Files, SearchGroup::Songs] {
        let search = scoped().with_group(group, None);
        assert_eq!(ScopedSearch::from_uri(&search.to_uri()).unwrap(), search);
    }
    for entity in [
        SearchEntityKey::Artist("björk & friends".into()),
        SearchEntityKey::Album {
            artist: "various artists".into(),
            album: "disc 1 = echoes?".into(),
        },
        SearchEntityKey::Album {
            artist: String::new(),
            album: "unknown performer".into(),
        },
    ] {
        let search = scoped().with_group(SearchGroup::Songs, Some(entity));
        let restored = ScopedSearch::from_uri(&search.to_uri()).unwrap();
        assert_eq!(restored, search);
        assert_eq!(restored.root, Path::new("/synthetic/music"));
    }
}

#[test]
fn old_scope_urls_default_to_all_physical_results() {
    let search = scoped();
    assert!(!search.to_uri().contains("group="));
    let restored = ScopedSearch::from_uri(&search.to_uri()).unwrap();
    assert_eq!(restored.group, SearchGroup::All);
    assert_eq!(restored.entity, None);
    assert_eq!(
        ScopedSearch::from_uri("search://old query").unwrap().group,
        SearchGroup::All
    );
}

#[test]
fn virtual_and_malformed_export_contexts_are_rejected() {
    let base = scoped().to_uri();
    for suffix in [
        "&group=albums",
        "&group=artists",
        "&group=files&artist-group=artist",
        "&artist-group=artist",
        "&group=songs&album-group=album",
        "&group=songs&artist-group=",
        "&group=songs&artist-group=artist&album-group=",
        "&group=songs&group=files",
        "&group=songs&artist-group=one&artist-group=two",
        "&group=songs&artist-group=%0A",
        "&group=songs&unknown=field",
    ] {
        assert!(
            ScopedSearch::from_uri(&format!("{base}{suffix}")).is_err(),
            "{suffix}"
        );
    }
}

#[test]
fn entity_names_normalize_unicode_but_remain_bounded() {
    assert_eq!(entity_field("İSTANBUL".into()).unwrap(), "i̇stanbul");
    assert!(entity_field("İ".repeat(512)).is_err());
    assert!(entity_field("a".repeat(2049)).is_err());
    assert!(entity_field("artist\0".into()).is_err());
    let search = scoped().with_group(
        SearchGroup::Songs,
        Some(SearchEntityKey::Artist("i̇".repeat(256))),
    );
    assert_eq!(ScopedSearch::from_uri(&search.to_uri()).unwrap(), search);
}
