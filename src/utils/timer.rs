// SPDX-License-Identifier: Apache-2.0
// Copyright (c) The pliron contributors

//! A simple timer utility.

use pliron::std_deps::time;

/// A simple timer utility
pub struct Timer(time::Instant);

impl Timer {
    /// Starts a new timer.
    pub fn start() -> Self {
        Self(time::Instant::now())
    }

    /// Returns the elapsed time since the timer was started.
    pub fn elapsed(&self) -> core::time::Duration {
        self.0.elapsed()
    }
}
