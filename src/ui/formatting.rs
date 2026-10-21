use std::cell::Cell;

use gtk::glib;

mod bindings;

#[cfg(test)]
mod tests;

pub(crate) use bindings::{bind, clear};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum DateFormat {
    #[default]
    Relative,
    Iso,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum SizeUnit {
    #[default]
    Decimal,
    Binary,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct Options {
    date: DateFormat,
    size: SizeUnit,
}

thread_local! {
    static OPTIONS: Cell<Options> = const { Cell::new(Options {
        date: DateFormat::Relative,
        size: SizeUnit::Decimal,
    }) };
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Value {
    Size(u64),
    Timestamp(i64),
    Selection(usize, u64),
    FreeSpace(u64),
}

pub(crate) fn configure(date: &str, size: &str) {
    let options = Options {
        date: if date == "iso" {
            DateFormat::Iso
        } else {
            DateFormat::Relative
        },
        size: if size == "binary" {
            SizeUnit::Binary
        } else {
            SizeUnit::Decimal
        },
    };
    if OPTIONS.replace(options) != options {
        bindings::refresh();
    }
}

fn date_format() -> DateFormat {
    OPTIONS.get().date
}

pub(crate) fn format_size(bytes: u64) -> String {
    format_size_with(bytes, OPTIONS.get().size)
}

fn format_size_with(bytes: u64, unit: SizeUnit) -> String {
    let (base, units) = match unit {
        SizeUnit::Decimal => (1000.0, ["B", "kB", "MB", "GB", "TB"]),
        SizeUnit::Binary => (1024.0, ["B", "KiB", "MiB", "GiB", "TiB"]),
    };
    if bytes < base as u64 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut index = 0;
    while value >= base && index < units.len() - 1 {
        value /= base;
        index += 1;
    }
    if (value * 10.0).round() / 10.0 >= base && index < units.len() - 1 {
        value /= base;
        index += 1;
    }
    let number = format!("{value:.1}");
    format!("{} {}", number.trim_end_matches(".0"), units[index])
}

pub(crate) fn format_timestamp(seconds: i64) -> String {
    let Ok(now) = glib::DateTime::now_local() else {
        return "—".into();
    };
    format_timestamp_at(seconds, date_format(), &now)
}

fn format_absolute(seconds: i64) -> String {
    glib::DateTime::from_unix_local(seconds)
        .and_then(|date| date.format("%Y-%m-%d %H:%M:%S %Z (%z)"))
        .map(|value| value.to_string())
        .unwrap_or_else(|_| "—".into())
}

fn date_text(date: &glib::DateTime, format: &str) -> String {
    date.format(format)
        .map(|value| value.to_string())
        .unwrap_or_else(|_| "—".into())
}

fn same_day(first: &glib::DateTime, second: &glib::DateTime) -> bool {
    first.year() == second.year() && first.day_of_year() == second.day_of_year()
}

fn format_timestamp_at(seconds: i64, format: DateFormat, now: &glib::DateTime) -> String {
    let Ok(date) =
        glib::DateTime::from_unix_utc(seconds).and_then(|date| date.to_timezone(&now.timezone()))
    else {
        return "—".into();
    };
    if format == DateFormat::Iso || seconds > now.to_unix() {
        return date_text(&date, "%Y-%m-%d %H:%M");
    }
    let elapsed = now.to_unix().saturating_sub(seconds);
    if elapsed < 60 {
        "Just now".into()
    } else if elapsed < 3600 {
        format!("{}m ago", elapsed / 60)
    } else if same_day(&date, now) {
        date_text(&date, "Today · %H:%M")
    } else if now
        .add_days(-1)
        .is_ok_and(|yesterday| same_day(&date, &yesterday))
    {
        date_text(&date, "Yesterday · %H:%M")
    } else {
        date_text(&date, "%Y-%m-%d %H:%M")
    }
}

pub(crate) fn format_value(value: Value) -> String {
    match value {
        Value::Size(bytes) => format_size(bytes),
        Value::Timestamp(seconds) => format_timestamp(seconds),
        Value::Selection(0, _) => String::new(),
        Value::Selection(count, bytes) => format!("{count} selected, {}", format_size(bytes)),
        Value::FreeSpace(bytes) => format!("{} free", format_size(bytes)),
    }
}
