use super::*;
use std::os::unix::fs::PermissionsExt;

#[test]
fn exported_art_is_bounded_private_unique_and_cleaned_up() {
    let bytes = b"\x89PNG\r\n\x1a\nfixture";
    let first = save_png(bytes).unwrap();
    let second = save_png(bytes).unwrap();
    assert_ne!(first.directory, second.directory);
    assert_eq!(
        fs::metadata(&first.directory).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(&first.path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(fs::read(&first.path).unwrap(), bytes);
    let path = first.directory.clone();
    drop(first);
    assert!(!path.exists());
    assert!(save_png(b"not a PNG").is_err());
    assert!(save_png(&vec![0; 1024 * 1024 + 1]).is_err());
}
