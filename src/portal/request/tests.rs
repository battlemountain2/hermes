use super::*;
use gtk::glib::variant::ToVariant;

fn parse(method: &str, options: Options) -> Result<Request, String> {
    Request::parse(method, "Choose a file".into(), String::new(), options)
}

#[test]
fn defaults_and_typed_options_follow_the_chooser_contract() {
    let request = parse("OpenFile", Options::new()).unwrap();
    assert_eq!(
        request.kind,
        Kind::Open {
            directory: false,
            multiple: false
        }
    );
    assert_eq!(request.accept_label, "Open");
    assert!(request.modal);
    let options = Options::from([
        ("directory".into(), true.to_variant()),
        ("multiple".into(), true.to_variant()),
        ("modal".into(), false.to_variant()),
        ("accept_label".into(), "Pick these".to_variant()),
        ("future_option".into(), 1_u32.to_variant()),
    ]);
    let request = parse("OpenFile", options).unwrap();
    assert_eq!(
        request.kind,
        Kind::Open {
            directory: true,
            multiple: true
        }
    );
    assert_eq!(request.accept_label, "Pick these");
    assert!(!request.modal);
    assert!(parse("Unknown", Options::new()).is_err());
    assert!(
        parse(
            "OpenFile",
            Options::from([("multiple".into(), "yes".to_variant())])
        )
        .is_err()
    );
}

#[test]
fn native_paths_preserve_bytes_and_reject_relative_unterminated_or_embedded_nul() {
    let request = parse(
        "SaveFile",
        Options::from([(
            "current_folder".into(),
            b"/tmp/byte\xff\0".to_vec().to_variant(),
        )]),
    )
    .unwrap();
    assert_eq!(
        request.folder.unwrap().as_os_str().as_encoded_bytes(),
        b"/tmp/byte\xff"
    );
    for bytes in [
        b"/tmp/missing-terminator".as_slice(),
        b"relative\0",
        b"/tmp/bad\0name\0",
        b"\0",
    ] {
        assert!(
            parse(
                "SaveFile",
                Options::from([("current_folder".into(), bytes.to_vec().to_variant())])
            )
            .is_err()
        );
    }
    for name in ["../bad", "folder/name", ".", "..", ""] {
        assert!(
            parse(
                "SaveFile",
                Options::from([("current_name".into(), name.to_variant())])
            )
            .is_err()
        );
    }
}

#[test]
fn save_files_requires_bounded_plain_names_and_preserves_non_utf8() {
    assert!(parse("SaveFiles", Options::new()).is_err());
    for names in [
        Vec::<Vec<u8>>::new(),
        vec![b"../bad\0".to_vec()],
        vec![b"ok\0".to_vec(); 65],
    ] {
        assert!(
            parse(
                "SaveFiles",
                Options::from([("files".into(), names.to_variant())])
            )
            .is_err()
        );
    }
    let request = parse(
        "SaveFiles",
        Options::from([(
            "files".into(),
            vec![b"byte\xff.txt\0".to_vec()].to_variant(),
        )]),
    )
    .unwrap();
    assert_eq!(
        request.kind,
        Kind::SaveFiles(vec![OsString::from_vec(b"byte\xff.txt".to_vec())])
    );
}

#[test]
fn filters_roundtrip_but_invalid_or_unbounded_rules_are_rejected() {
    let filter: Filter = (
        "Pictures".into(),
        vec![(0, "*.png".into()), (1, "image/jpeg".into())],
    );
    let request = parse(
        "OpenFile",
        Options::from([
            ("filters".into(), vec![filter.clone()].to_variant()),
            ("current_filter".into(), filter.clone().to_variant()),
        ]),
    )
    .unwrap();
    assert_eq!(request.filters, [filter.clone()]);
    assert_eq!(request.current_filter, Some(filter));
    for rules in [
        vec![(2, "*.png".into())],
        vec![(0, "***".into())],
        vec![(1, "not-a-mime".into())],
        vec![(0, "*.png".into()); 257],
    ] {
        let filter: Filter = ("Invalid".into(), rules);
        assert!(
            parse(
                "OpenFile",
                Options::from([("filters".into(), vec![filter].to_variant())])
            )
            .is_err()
        );
    }
}

#[test]
fn choice_ids_values_and_defaults_are_unique_valid_and_bounded() {
    let boolean: Choice = ("readonly".into(), "Read only".into(), vec![], "true".into());
    let enumeration: Choice = (
        "encoding".into(),
        "Encoding".into(),
        vec![("utf8".into(), "UTF-8".into())],
        "utf8".into(),
    );
    let valid = vec![boolean.clone(), enumeration];
    assert_eq!(
        parse(
            "OpenFile",
            Options::from([("choices".into(), valid.to_variant())])
        )
        .unwrap()
        .choices,
        valid
    );
    for choices in [
        vec![boolean.clone(), boolean.clone()],
        vec![("invalid".into(), "Invalid".into(), vec![], "maybe".into())],
        vec![boolean; 17],
    ] {
        assert!(
            parse(
                "OpenFile",
                Options::from([("choices".into(), choices.to_variant())])
            )
            .is_err()
        );
    }
}

#[test]
fn titles_parent_handles_and_control_characters_are_bounded() {
    for parent in [
        "not-a-window".to_owned(),
        "x11:bad\nwindow".to_owned(),
        "wayland:".to_owned() + &"x".repeat(4096),
    ] {
        assert!(Request::parse("OpenFile", "Open".into(), parent, Options::new()).is_err());
    }
    assert!(Request::parse("OpenFile", "x".repeat(4097), String::new(), Options::new()).is_err());
    assert!(
        Request::parse(
            "OpenFile",
            "Open".into(),
            "wayland:generated".into(),
            Options::new()
        )
        .is_ok()
    );
}
