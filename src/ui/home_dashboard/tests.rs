// SPDX-License-Identifier: GPL-3.0-or-later

use super::*;

#[test]
fn test_same_day_matching() {
    let now = glib::DateTime::now_local().expect("local time");
    assert!(same_day(&now, &now));

    if let Ok(yesterday) = now.add_days(-1) {
        assert!(!same_day(&now, &yesterday));
    }
}

#[test]
fn test_date_bucket_categorization() {
    let now = glib::DateTime::now_local().expect("local time");
    let now_unix = now.to_unix();

    // Now -> Today
    assert_eq!(DateBucket::from_timestamp(now_unix, &now), DateBucket::Today);

    // 2 hours ago -> Today
    assert_eq!(
        DateBucket::from_timestamp(now_unix - 7200, &now),
        DateBucket::Today
    );

    // Yesterday
    if let Ok(yesterday) = now.add_days(-1) {
        let yesterday_unix = yesterday.to_unix();
        assert_eq!(
            DateBucket::from_timestamp(yesterday_unix, &now),
            DateBucket::Yesterday
        );
    }

    // 4 days ago -> Earlier this week
    if let Ok(four_days_ago) = now.add_days(-4) {
        let four_days_unix = four_days_ago.to_unix();
        assert_eq!(
            DateBucket::from_timestamp(four_days_unix, &now),
            DateBucket::EarlierThisWeek
        );
    }

    // 30 days ago -> Older
    if let Ok(thirty_days_ago) = now.add_days(-30) {
        let thirty_days_unix = thirty_days_ago.to_unix();
        assert_eq!(
            DateBucket::from_timestamp(thirty_days_unix, &now),
            DateBucket::Older
        );
    }
}

#[test]
fn test_quick_access_contains_personal_folder() {
    let folders = load_quick_access_folders();
    assert!(!folders.is_empty());
    assert_eq!(folders[0].name, "Personal Folder");
    assert_eq!(folders[0].path_display, "~");
    assert_eq!(folders[0].icon, icons::HOME);
}
