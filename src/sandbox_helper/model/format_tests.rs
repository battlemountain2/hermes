use super::*;
use std::os::fd::AsRawFd;

#[test]
fn declared_model_format_not_descriptor_name_selects_the_parser() {
    let input = fs::File::open("/dev/null").expect("empty input descriptor");
    let path = format!("/proc/self/fd/{}", input.as_raw_fd());
    assert!(
        render_model_with_format(Path::new(&path), 64, ModelFormat::ThreeMf)
            .expect_err("empty 3MF")
            .contains("3MF archive")
    );
    assert_eq!(
        render_model_with_format(Path::new(&path), 64, ModelFormat::FreeCad)
            .expect_err("unsupported explicit format"),
        "Unsupported model format"
    );
}

#[test]
fn explicit_three_mf_preserves_archive_entry_limit_before_rendering() {
    let directory = crate::sandbox::browser::format_tests::FixtureDirectory::new();
    let path = directory.0.join("opaque.fd");
    let file = fs::File::create(&path).expect("fixture archive");
    let mut archive = zip::ZipWriter::new(file);
    for index in 0..=MAX_3MF_ARCHIVE_ENTRIES {
        archive
            .start_file(
                format!("entry-{index}"),
                zip::write::SimpleFileOptions::default(),
            )
            .expect("fixture archive member");
    }
    archive.finish().expect("finished fixture");
    assert_eq!(
        render_model_with_format(&path, 64, ModelFormat::ThreeMf).expect_err("entry limit"),
        "3MF archive contains too many entries"
    );
}
