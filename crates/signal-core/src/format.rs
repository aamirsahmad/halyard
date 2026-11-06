//! Human readable time formatting following Signal Desktop's conventions.

use chrono::{DateTime, Datelike, Local, NaiveDate, TimeZone};

use crate::model::Millis;

fn to_local<Tz: TimeZone>(ms: Millis, tz: &Tz) -> DateTime<Tz> {
    tz.timestamp_millis_opt(ms)
        .single()
        .unwrap_or_else(|| tz.timestamp_millis_opt(0).unwrap())
}

/// Short timestamp for the conversation list (`Now`, `5m`, `14:02`, `Mon`, `Mar 3`, `3/4/2023`).
pub fn conversation_list_time(ts: Millis, now: Millis) -> String {
    conversation_list_time_tz(ts, now, &Local)
}

pub fn conversation_list_time_tz<Tz: TimeZone>(ts: Millis, now: Millis, tz: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    if ts <= 0 {
        return String::new();
    }
    let diff = now - ts;
    if diff < 60_000 {
        return "Now".into();
    }
    if diff < 60 * 60_000 {
        return format!("{}m", diff / 60_000);
    }
    let t = to_local(ts, tz);
    let n = to_local(now, tz);
    if t.date_naive() == n.date_naive() {
        return t.format("%H:%M").to_string();
    }
    let days = (n.date_naive() - t.date_naive()).num_days();
    if (0..7).contains(&days) {
        return t.format("%a").to_string();
    }
    if t.year() == n.year() {
        return t.format("%b %-d").to_string();
    }
    t.format("%-m/%-d/%Y").to_string()
}

/// Time shown under a message bubble (`Now`, `5m`, `14:02`, `Mon 14:02`, `Mar 3, 14:02`).
pub fn message_time(ts: Millis, now: Millis) -> String {
    message_time_tz(ts, now, &Local)
}

pub fn message_time_tz<Tz: TimeZone>(ts: Millis, now: Millis, tz: &Tz) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let diff = now - ts;
    if (0..60_000).contains(&diff) {
        return "Now".into();
    }
    if (0..60 * 60_000).contains(&diff) {
        return format!("{}m", diff / 60_000);
    }
    let t = to_local(ts, tz);
    let n = to_local(now, tz);
    t.format("%H:%M").to_string().pipe(|hm| {
        if t.date_naive() == n.date_naive() {
            hm
        } else if (0..7).contains(&(n.date_naive() - t.date_naive()).num_days()) {
            format!("{} {hm}", t.format("%a"))
        } else {
            format!("{}, {hm}", t.format("%b %-d"))
        }
    })
}

/// Label of the date separator shown in the timeline.
pub fn date_separator(day: NaiveDate, today: NaiveDate) -> String {
    let days = (today - day).num_days();
    match days {
        0 => "Today".into(),
        1 => "Yesterday".into(),
        2..=6 => day.format("%A").to_string(),
        _ if day.year() == today.year() => day.format("%a, %b %-d").to_string(),
        _ => day.format("%a, %b %-d, %Y").to_string(),
    }
}

pub fn local_day(ts: Millis) -> NaiveDate {
    to_local(ts, &Local).date_naive()
}

/// Human readable byte size.
pub fn file_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut v = bytes as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", UNITS[unit])
    }
}

trait Pipe: Sized {
    fn pipe<R>(self, f: impl FnOnce(Self) -> R) -> R {
        f(self)
    }
}
impl<T> Pipe for T {}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn ms(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> Millis {
        Utc.with_ymd_and_hms(y, mo, d, h, mi, 0)
            .unwrap()
            .timestamp_millis()
    }

    #[test]
    fn list_time() {
        let now = ms(2026, 11, 5, 12, 0); // Thursday
        let f = |ts| conversation_list_time_tz(ts, now, &Utc);
        assert_eq!(f(0), "");
        assert_eq!(f(now - 10_000), "Now");
        assert_eq!(f(now - 5 * 60_000), "5m");
        assert_eq!(f(ms(2026, 11, 5, 8, 3)), "08:03");
        assert_eq!(f(ms(2026, 11, 2, 8, 3)), "Mon");
        assert_eq!(f(ms(2026, 3, 3, 8, 3)), "Mar 3");
        assert_eq!(f(ms(2024, 3, 4, 8, 3)), "3/4/2024");
    }

    #[test]
    fn msg_time() {
        let now = ms(2026, 11, 5, 12, 0);
        let f = |ts| message_time_tz(ts, now, &Utc);
        assert_eq!(f(now), "Now");
        assert_eq!(f(now - 59 * 60_000), "59m");
        assert_eq!(f(ms(2026, 11, 5, 9, 30)), "09:30");
        assert_eq!(f(ms(2026, 11, 3, 9, 30)), "Tue 09:30");
        assert_eq!(f(ms(2026, 1, 3, 9, 30)), "Jan 3, 09:30");
        // clock skew: a message "from the future" still renders a time
        assert_eq!(f(now + 3 * 3_600_000), "15:00");
    }

    #[test]
    fn separators() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 5).unwrap();
        let d = |y, m, day| NaiveDate::from_ymd_opt(y, m, day).unwrap();
        assert_eq!(date_separator(today, today), "Today");
        assert_eq!(date_separator(d(2026, 11, 4), today), "Yesterday");
        assert_eq!(date_separator(d(2026, 11, 2), today), "Monday");
        assert_eq!(date_separator(d(2026, 10, 1), today), "Thu, Oct 1");
        assert_eq!(date_separator(d(2025, 10, 1), today), "Wed, Oct 1, 2025");
    }

    #[test]
    fn sizes() {
        assert_eq!(file_size(512), "512 B");
        assert_eq!(file_size(2048), "2.0 KB");
        assert_eq!(file_size(5 * 1024 * 1024 + 1), "5.0 MB");
    }
}
