use super::*;

fn datetime(zone: &str, year: i32, month: i32, day: i32, hour: i32, minute: i32) -> glib::DateTime {
    let timezone = glib::TimeZone::from_identifier(Some(zone)).expect("fixture timezone");
    glib::DateTime::new(&timezone, year, month, day, hour, minute, 0.0).unwrap()
}

#[test]
fn decimal_sizes_preserve_precision_and_promote_rounded_boundaries() {
    for (bytes, expected) in [
        (0, "0 B"),
        (999, "999 B"),
        (1_000, "1 kB"),
        (1_024, "1 kB"),
        (1_200, "1.2 kB"),
        (999_900, "999.9 kB"),
        (999_999, "1 MB"),
        (1_000_000, "1 MB"),
        (2_500_000_000, "2.5 GB"),
        (10_000_000_000_000, "10 TB"),
        (u64::MAX, "18446744.1 TB"),
    ] {
        assert_eq!(format_size_with(bytes, SizeUnit::Decimal), expected);
    }
}

#[test]
fn binary_sizes_use_iec_units_and_preserve_byte_values_below_one_kib() {
    for (bytes, expected) in [
        (0, "0 B"),
        (1_000, "1000 B"),
        (1_023, "1023 B"),
        (1_024, "1 KiB"),
        (1_536, "1.5 KiB"),
        (1_048_575, "1 MiB"),
        (1_048_576, "1 MiB"),
        (1_500_000, "1.4 MiB"),
        (1_073_741_824, "1 GiB"),
        (u64::MAX, "16777216 TiB"),
    ] {
        assert_eq!(format_size_with(bytes, SizeUnit::Binary), expected);
    }
}

#[test]
fn relative_dates_cover_minutes_calendar_days_and_older_files() {
    let now = datetime("UTC", 2026, 10, 18, 12, 0);
    for (elapsed, expected) in [
        (0, "Just now"),
        (59, "Just now"),
        (60, "1m ago"),
        (240, "4m ago"),
        (3599, "59m ago"),
        (3600, "Today · 11:00"),
        (86400, "Yesterday · 12:00"),
        (172800, "2026-10-16 12:00"),
    ] {
        assert_eq!(
            format_timestamp_at(now.to_unix() - elapsed, DateFormat::Relative, &now),
            expected
        );
    }
}

#[test]
fn relative_dates_use_calendar_yesterday_at_midnight_and_year_boundaries() {
    for (now, previous, expected) in [
        (
            datetime("UTC", 2026, 10, 18, 0, 30),
            datetime("UTC", 2026, 10, 17, 23, 0),
            "Yesterday · 23:00",
        ),
        (
            datetime("UTC", 2026, 1, 1, 12, 0),
            datetime("UTC", 2025, 12, 31, 12, 0),
            "Yesterday · 12:00",
        ),
    ] {
        assert_eq!(
            format_timestamp_at(previous.to_unix(), DateFormat::Relative, &now),
            expected
        );
    }
}

#[test]
fn daylight_saving_days_are_not_assumed_to_have_twenty_four_hours() {
    for (month, today, previous, hours) in [(3, 9, 8, 23), (11, 2, 1, 25)] {
        let now = datetime("America/Denver", 2026, month, today, 0, 30);
        let yesterday = datetime("America/Denver", 2026, month, previous, 0, 30);
        assert_eq!(now.to_unix() - yesterday.to_unix(), hours * 3600);
        assert_eq!(
            format_timestamp_at(yesterday.to_unix(), DateFormat::Relative, &now),
            "Yesterday · 00:30"
        );
    }
}

#[test]
fn absolute_and_future_dates_remain_unambiguous_and_use_the_requested_timezone() {
    let now = datetime("Asia/Tokyo", 2026, 10, 18, 12, 0);
    let date = datetime("UTC", 2026, 10, 18, 2, 0);
    assert_eq!(
        format_timestamp_at(date.to_unix(), DateFormat::Iso, &now),
        "2026-10-18 11:00"
    );
    assert_eq!(
        format_timestamp_at(now.to_unix() + 300, DateFormat::Relative, &now),
        "2026-10-18 12:05"
    );
    assert_eq!(
        format_timestamp_at(-1, DateFormat::Iso, &now),
        "1970-01-01 08:59"
    );
}

#[test]
fn invalid_timestamps_are_bounded_and_do_not_panic() {
    let now = datetime("UTC", 2026, 10, 18, 12, 0);
    for seconds in [i64::MIN, i64::MAX] {
        for format in [DateFormat::Relative, DateFormat::Iso] {
            assert_eq!(format_timestamp_at(seconds, format, &now), "—");
        }
        assert_eq!(format_absolute(seconds), "—");
    }
}

#[test]
fn cached_options_apply_without_initializing_gtk_or_loading_preferences() {
    struct Restore(Options);
    impl Drop for Restore {
        fn drop(&mut self) {
            OPTIONS.set(self.0);
        }
    }
    let _restore = Restore(OPTIONS.get());
    configure("iso", "binary");
    assert_eq!(date_format(), DateFormat::Iso);
    assert_eq!(format_size(1536), "1.5 KiB");
    assert_eq!(format_value(Value::Selection(0, 1536)), "");
    assert_eq!(
        format_value(Value::Selection(2, 1536)),
        "2 selected, 1.5 KiB"
    );
    assert_eq!(format_value(Value::FreeSpace(1536)), "1.5 KiB free");
    configure("obsolete-date-style", "obsolete-size-units");
    assert_eq!(date_format(), DateFormat::Relative);
    assert_eq!(format_size(1536), "1.5 kB");
}
