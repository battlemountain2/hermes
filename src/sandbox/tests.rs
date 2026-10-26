// SPDX-License-Identifier: GPL-3.0-or-later

use std::path::Path;

use super::{Cancellation, ParseOperation, parse, sandbox_command};

#[test]
fn audio_preview_metadata_is_json_with_unchanged_sandbox_limits() {
    let operation = ParseOperation::PreviewAudioMetadata;
    let command = sandbox_command(
        Path::new("/tmp/strata"),
        Path::new("/tmp/track.flac"),
        Path::new("/tmp/output"),
        operation,
        0,
    ).unwrap();
    let arguments = command
        .get_args()
        .map(|value| value.to_string_lossy())
        .collect::<Vec<_>>();
    assert_eq!(operation.output_name(), "result.json");
    for expected in [
        "preview-audio-metadata",
        "/output/result.json",
        "--unshare-all",
        "--as=1342177280",
        "--cpu=10",
        "--fsize=33554432",
    ] {
        assert!(
            arguments.iter().any(|value| value == expected),
            "Missing {expected}"
        );
    }
}

#[test]
fn renderer_failures_include_bounded_decoder_details() {
    use std::os::unix::process::ExitStatusExt;
    let status = std::process::ExitStatus::from_raw(256);
    let message =
        super::renderer_failure_message(status, b"Invalid data found when processing input\n");
    assert!(message.contains("exit status: 1"));
    assert!(message.contains("Invalid data found"));
    let message = super::renderer_failure_message(status, &vec![b'x'; 10000]);
    assert!(message.len() < 2200);
    assert!(super::renderer_failure_message(status, b"").contains("resource limit"));
    assert!(!super::renderer_failure_message(status, b"bad\0data\x1b").contains('\0'));
}

#[test]
fn sandbox_exposes_only_runtime_input_and_private_output() {
    let command = sandbox_command(
        Path::new("/tmp/strata"),
        Path::new("/home/alice/Downloads/untrusted.pdf"),
        Path::new("/tmp/private-output"),
        ParseOperation::PreviewPdf,
        2,
    ).unwrap();
    let arguments: Vec<_> = command
        .get_args()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect();
    let joined = arguments.join(" ");

    assert!(joined.contains("--unshare-all"));
    assert!(joined.contains("--clearenv"));
    assert!(joined.contains("--ro-bind /home/alice/Downloads/untrusted.pdf /input.pdf"));
    assert!(joined.contains("--preview-helper preview-pdf /input.pdf /output/result.png"));
    assert!(joined.contains("--bind /tmp/private-output /output"));
    assert!(joined.contains("--as=1342177280"));
    assert!(joined.contains("--cpu=10"));
    assert!(joined.contains("--fsize=33554432"));
    // RLIMIT_NPROC counts every process owned by the host user, not just the
    // sandbox, and can prevent legitimate media decoders from starting.
    assert!(!joined.contains("--nproc"));
    assert!(!joined.contains("--ro-bind /home /home"));
    assert!(!joined.contains("--share-net"));
}

#[test]
fn sandbox_keeps_only_a_bounded_safe_format_suffix() {
    use std::{ffi::OsStr, os::unix::ffi::OsStrExt};
    for (name, expected) in [
        ("private-name.DOCX", "/input.docx"),
        ("private-name.3MF", "/input.3mf"),
        ("private-name.odt", "/input.odt"),
        ("private-name", "/input"),
        ("private-name.bad suffix", "/input"),
        ("private-name.abcdefghijklmnopq", "/input"),
    ] {
        assert_eq!(super::sandbox_input_path(Path::new(name)), expected);
    }
    assert_eq!(
        super::sandbox_input_path(Path::new(OsStr::from_bytes(b"private.\xff"))),
        "/input"
    );
}

#[test]
fn one_shot_renderers_reuse_only_optional_read_only_system_font_caches() {
    for operation in [
        ParseOperation::ConvertOfficePdf,
        ParseOperation::ThumbnailOffice,
        ParseOperation::PreviewPdf,
        ParseOperation::ThumbnailPdf,
    ] {
        let command = sandbox_command(
            Path::new("/generated/hermes"),
            Path::new("/generated/input.pdf"),
            Path::new("/generated/output"),
            operation,
            0,
        )
        .unwrap();
        let arguments = command
            .get_args()
            .map(|argument| argument.to_str().unwrap())
            .collect::<Vec<_>>();
        assert!(arguments.windows(3).any(|values| values
            == [
                "--ro-bind-try",
                "/var/cache/fontconfig",
                "/var/cache/fontconfig",
            ]));
        assert_eq!(
            arguments
                .iter()
                .filter(|argument| argument.starts_with("/var/"))
                .count(),
            2
        );
        assert!(
            arguments
                .windows(3)
                .any(|values| values == ["--setenv", "HOME", "/nonexistent"])
        );
        assert!(
            arguments
                .windows(3)
                .any(|values| values == ["--setenv", "XDG_CACHE_HOME", "/tmp/cache"])
        );
        assert!(arguments.contains(&"--unshare-all"));
        assert!(!arguments.contains(&"--share-net"));
    }
}

#[test]
#[ignore = "Requires bubblewrap and a built helper; generated office documents only"]
fn one_shot_office_previews_preserve_format_and_text_limits() {
    use crate::sandbox::browser::format_tests::{FixtureDirectory, write_package};
    let directory = FixtureDirectory::new();
    let helper = super::helper_executable().unwrap();
    for (name, member, xml) in [
        (
            "private.DOCX",
            "word/document.xml",
            "<w:document><w:p><w:t>Hermes generated office document</w:t></w:p></w:document>",
        ),
        (
            "private.ODT",
            "content.xml",
            "<office:document-content><text:p>Hermes generated office document</text:p></office:document-content>",
        ),
    ] {
        let path = directory.0.join(name);
        write_package(&path, &[(member, xml.as_bytes())]);
        let result = super::parse_one_shot_with_executable(
            &path,
            ParseOperation::PreviewOffice,
            1024,
            &Cancellation::default(),
            &helper,
        )
        .unwrap();
        assert!(
            String::from_utf8(result.data)
                .unwrap()
                .contains("Hermes generated office document")
        );
        let limited = super::parse_one_shot_with_executable(
            &path,
            ParseOperation::ExtractOfficeText,
            8,
            &Cancellation::default(),
            &helper,
        )
        .unwrap();
        assert!(limited.data.len() <= 8);
        let cancellation = Cancellation::default();
        cancellation.cancel();
        assert!(super::parse(&path, ParseOperation::PreviewOffice, 0, &cancellation).is_err());
    }
    let slides = directory.0.join("generated.PPTX");
    write_package(&slides, &[
        ("ppt/presentation.xml", b"<presentation><sldId r:id=\"first\"/></presentation>"),
        ("ppt/_rels/presentation.xml.rels", b"<Relationships><Relationship Id=\"first\" Type=\"internal/slide\" Target=\"slides/slide1.xml\"/></Relationships>"),
        ("ppt/slides/slide1.xml", b"<sld><p><t>Generated slide body</t></p></sld>"),
    ]);
    let result = super::parse_one_shot_with_executable(
        &slides,
        ParseOperation::PreviewOffice,
        1024,
        &Cancellation::default(),
        &helper,
    )
    .unwrap();
    let text = String::from_utf8(result.data).unwrap();
    assert!(text.contains("Slide 1"));
    assert!(text.contains("Generated slide body"));
}

#[test]
fn cancelled_requests_fail_without_starting_a_renderer() {
    let cancellation = Cancellation::default();
    cancellation.cancel();
    let error = parse(
        Path::new("does-not-need-to-exist"),
        ParseOperation::PreviewImage,
        0,
        &cancellation,
    )
    .err()
    .expect("cancelled parse must fail");

    assert_eq!(error, "Preview cancelled");
}

#[test]
fn pdf_text_extraction_uses_a_text_output_inside_the_sandbox() {
    let operation = ParseOperation::ExtractPdfText;
    let command = sandbox_command(
        Path::new("/tmp/strata"),
        Path::new("/home/alice/Documents/manual.pdf"),
        Path::new("/tmp/private-output"),
        operation,
        1_048_576,
    ).unwrap();
    let arguments: Vec<_> = command
        .get_args()
        .map(|argument| argument.to_string_lossy().into_owned())
        .collect();

    assert!(
        arguments
            .iter()
            .any(|argument| argument == "extract-pdf-text")
    );
    assert!(
        arguments
            .iter()
            .any(|argument| argument == "/output/result.txt")
    );
    assert!(arguments.iter().any(|argument| argument == "1048576"));
}

#[test]
fn archive_and_office_operations_use_bounded_text_outputs() {
    for (operation, argument) in [
        (ParseOperation::PreviewArchive, "preview-archive"),
        (ParseOperation::PreviewOffice, "preview-office"),
        (ParseOperation::ExtractOfficeText, "extract-office-text"),
    ] {
        let command = sandbox_command(
            Path::new("/tmp/strata"),
            Path::new("/home/alice/Documents/input.bin"),
            Path::new("/tmp/private-output"),
            operation,
            1_048_576,
        ).unwrap();
        let arguments = command
            .get_args()
            .map(|value| value.to_string_lossy())
            .collect::<Vec<_>>();
        assert!(arguments.iter().any(|value| value == argument));
        assert!(arguments.iter().any(|value| value == "/output/result.txt"));
    }
}
