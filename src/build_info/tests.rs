// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

#[test]
fn package_metadata_is_available_to_the_about_page() {
    assert!(!DESCRIPTION.is_empty());
    assert!(!VERSION.is_empty());
    assert!(!COMMIT.is_empty());
    assert_eq!(REPOSITORY, "https://github.com/battlemountain2/hermes");
    assert_eq!(AUTHOR, "battlemountain2");
}
