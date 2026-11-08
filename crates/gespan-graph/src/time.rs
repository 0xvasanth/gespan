use std::time::{SystemTime, UNIX_EPOCH};

/// A point in time as Unix epoch **milliseconds**. Every time in the graph
/// is one of these, and nothing converts or guesses units: a caller with
/// seconds multiplies them itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Timestamp(i64);

impl Timestamp {
    pub const fn from_millis(ms: i64) -> Timestamp {
        Timestamp(ms)
    }

    /// `None` before the epoch, or past what `i64` milliseconds can hold.
    pub fn from_system_time(time: SystemTime) -> Option<Timestamp> {
        let since = time.duration_since(UNIX_EPOCH).ok()?;
        i64::try_from(since.as_millis()).ok().map(Timestamp)
    }

    pub const fn as_millis(self) -> i64 {
        self.0
    }
}
