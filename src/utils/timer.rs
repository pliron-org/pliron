//! A simple timer utility.

/// A simple timer utility
pub struct Timer(pliron::deps::time::Instant);

impl Timer {
    /// Starts a new timer.
    pub fn start() -> Self {
        Self(pliron::deps::time::Instant::now())
    }

    /// Returns the elapsed time since the timer was started.
    pub fn elapsed(&self) -> core::time::Duration {
        self.0.elapsed()
    }
}
