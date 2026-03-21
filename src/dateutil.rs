//! Shared date/time utilities.
//!
//! Epoch-to-date conversion without external dependencies (no chrono).
//! Used by the knowledge store, maintenance, and other modules.

/// Format the current UTC time as "YYYY-MM-DD".
pub fn today_string() -> String {
    let secs = epoch_secs();
    date_from_epoch(secs)
}

/// Format the current UTC time as "YYYY-MM-DD HH:MM:SS".
pub fn datetime_now() -> String {
    let secs = epoch_secs();
    let (y, m, d) = ymd_from_epoch(secs);
    let secs_today = secs % 86400;
    let hours = secs_today / 3600;
    let minutes = (secs_today % 3600) / 60;
    let seconds = secs_today % 60;
    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", y, m, d, hours, minutes, seconds)
}

fn epoch_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn date_from_epoch(secs: u64) -> String {
    let (y, m, d) = ymd_from_epoch(secs);
    format!("{:04}-{:02}-{:02}", y, m, d)
}

fn ymd_from_epoch(secs: u64) -> (i64, usize, i64) {
    let days = secs / 86400;
    let mut y = 1970i64;
    let mut remaining = days as i64;
    loop {
        let days_in_year = if is_leap(y) { 366 } else { 365 };
        if remaining < days_in_year { break; }
        remaining -= days_in_year;
        y += 1;
    }
    let leap = is_leap(y);
    let month_days = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut m = 0;
    for (i, &md) in month_days.iter().enumerate() {
        if remaining < md as i64 { m = i + 1; break; }
        remaining -= md as i64;
    }
    (y, m, remaining + 1)
}

fn is_leap(y: i64) -> bool {
    y % 4 == 0 && (y % 100 != 0 || y % 400 == 0)
}
