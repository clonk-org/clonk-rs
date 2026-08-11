use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

use chrono::Local;

static BASE_TIME: OnceLock<SystemTime> = OnceLock::new();

/// Rust port of `timeGetTime`.
/// Returns milliseconds elapsed since the first invocation,
/// matching the non-Windows implementation that uses a lazily
/// initialised offset.
pub fn time_get_time() -> u64 {
    let base_time = BASE_TIME.get_or_init(SystemTime::now);
    match SystemTime::now().duration_since(*base_time) {
        Ok(duration) => duration.as_millis() as u64,
        Err(err) => {
            // Clock went backwards; saturate at zero similar to unsigned wrap in C++.
            duration_saturating_sub(Duration::ZERO, err.duration())
        }
    }
}

fn duration_saturating_sub(base: Duration, sub: Duration) -> u64 {
    if sub >= base {
        0
    } else {
        (base - sub).as_millis() as u64
    }
}

/// Returns a timestamp in the `[HH:MM:SS]` format wrapped with
/// markup colour tags when requested, reproducing `GetCurrentTimeStamp`.
pub fn current_timestamp(enable_markup_color: bool) -> String {
    let now = Local::now();
    let time_part = now.format("[%H:%M:%S]").to_string();
    if enable_markup_color {
        format!("<c 909090>{}</c>", time_part)
    } else {
        time_part
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_format_with_markup() {
        let timestamp = current_timestamp(true);
        assert!(timestamp.starts_with("<c 909090>"), "{}", timestamp);
        assert!(timestamp.ends_with("</c>"));
        assert_eq!(timestamp.len(), "<c 909090>".len() + "</c>".len() + 10);
    }

    #[test]
    fn timestamp_format_plain() {
        let timestamp = current_timestamp(false);
        assert!(timestamp.starts_with('['));
        assert!(timestamp.ends_with(']'));
        assert_eq!(timestamp.len(), 10);
    }

    #[test]
    fn time_get_time_monotonic() {
        let first = time_get_time();
        let second = time_get_time();
        assert!(second >= first);
    }
}
