use super::*;
use std::os::fd::AsRawFd;

#[test]
fn declared_cover_format_not_descriptor_name_selects_the_parser() {
    let input = fs::File::open("/dev/null").expect("empty input descriptor");
    let path = format!("/proc/self/fd/{}", input.as_raw_fd());
    assert!(CoverFormat::for_path(Path::new(&path)).is_none());
    assert!(
        render_cover_with_format(Path::new(&path), 64, CoverFormat::Epub)
            .expect_err("empty EPUB")
            .contains("Invalid EPUB archive")
    );
    assert!(
        render_cover_with_format(Path::new(&path), 64, CoverFormat::Cbz)
            .expect_err("empty CBZ")
            .contains("Invalid CBZ archive")
    );
}
