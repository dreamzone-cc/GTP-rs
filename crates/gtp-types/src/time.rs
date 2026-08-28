use core::fmt;
use core::ops::{Add, AddAssign, Sub};

#[cfg(feature = "std")]
use std::sync::OnceLock;
#[cfg(feature = "std")]
use std::time::Instant;

#[cfg(feature = "std")]
static BASE_INSTANT: OnceLock<Instant> = OnceLock::new();

/// High-resolution monotonic timestamp in microseconds.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Default)]
pub struct MonotonicTime(pub u64);

impl MonotonicTime {
    pub const ZERO: Self = Self(0);

    pub const fn from_micros(micros: u64) -> Self {
        Self(micros)
    }

    pub const fn as_micros(self) -> u64 {
        self.0
    }

    pub const fn as_millis(self) -> u64 {
        self.0 / 1_000
    }

    #[cfg(feature = "std")]
    pub fn now() -> Self {
        let base = BASE_INSTANT.get_or_init(Instant::now);
        let elapsed = Instant::now().duration_since(*base);
        Self(elapsed.as_micros() as u64)
    }

    pub fn duration_since(self, earlier: MonotonicTime) -> Duration {
        Duration::from_micros(self.0.saturating_sub(earlier.0))
    }

    pub fn saturating_sub_duration(self, d: Duration) -> MonotonicTime {
        MonotonicTime(self.0.saturating_sub(d.as_micros()))
    }
}

impl fmt::Debug for MonotonicTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}us", self.0)
    }
}

impl Add<Duration> for MonotonicTime {
    type Output = MonotonicTime;

    fn add(self, rhs: Duration) -> Self::Output {
        MonotonicTime(self.0.saturating_add(rhs.as_micros()))
    }
}

impl AddAssign<Duration> for MonotonicTime {
    fn add_assign(&mut self, rhs: Duration) {
        self.0 = self.0.saturating_add(rhs.as_micros());
    }
}

impl Sub<MonotonicTime> for MonotonicTime {
    type Output = Duration;

    fn sub(self, rhs: MonotonicTime) -> Self::Output {
        Duration::from_micros(self.0.saturating_sub(rhs.0))
    }
}

/// Duration in microseconds.
#[derive(Copy, Clone, Eq, PartialEq, Ord, PartialOrd, Hash, Default)]
pub struct Duration(pub u64);

impl Duration {
    pub const ZERO: Self = Self(0);

    pub const fn from_micros(micros: u64) -> Self {
        Self(micros)
    }

    pub const fn from_millis(millis: u64) -> Self {
        Self(millis.saturating_mul(1_000))
    }

    pub const fn from_secs(secs: u64) -> Self {
        Self(secs.saturating_mul(1_000_000))
    }

    pub const fn as_micros(self) -> u64 {
        self.0
    }

    pub const fn as_millis(self) -> u64 {
        self.0 / 1_000
    }

    pub fn as_secs_f64(self) -> f64 {
        self.0 as f64 / 1_000_000.0
    }
}

impl fmt::Debug for Duration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0 >= 1_000_000 {
            write!(f, "{:.3}s", self.as_secs_f64())
        } else if self.0 >= 1_000 {
            write!(f, "{:.3}ms", self.0 as f64 / 1_000.0)
        } else {
            write!(f, "{}us", self.0)
        }
    }
}

impl Add for Duration {
    type Output = Duration;

    fn add(self, rhs: Self) -> Self::Output {
        Duration(self.0.saturating_add(rhs.0))
    }
}

impl Sub for Duration {
    type Output = Duration;

    fn sub(self, rhs: Self) -> Self::Output {
        Duration(self.0.saturating_sub(rhs.0))
    }
}
