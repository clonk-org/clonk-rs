use std::cell::Cell;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct CaptionScrollState {
    last_change: Option<Instant>,
    position: i32,
    direction: i8,
}

pub(crate) fn advance_caption_scroll(
    state: &Cell<CaptionScrollState>,
    now: Instant,
    max_scroll: i32,
    delay: Duration,
) -> i32 {
    let mut current = state.get();
    let Some(last_change) = current.last_change else {
        current.last_change = Some(now);
        state.set(current);
        return 0;
    };
    if now.checked_duration_since(last_change).unwrap_or_default() >= delay {
        if current.direction == 0 {
            current.direction = 1;
        }
        if max_scroll > 0 {
            current.position += i32::from(current.direction);
            if current.position >= max_scroll || current.position < 0 {
                current.direction = -current.direction;
                current.position += i32::from(current.direction);
                current.last_change = Some(now);
            }
        }
    }
    state.set(current);
    current.position
}
