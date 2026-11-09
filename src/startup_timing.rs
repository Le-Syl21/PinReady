//! Startup phase timing. The launcher builds its table list before the window
//! exists, so anything slow in that path delays the first frame; these logs
//! say where the time goes.

use std::sync::OnceLock;
use std::time::Instant;

static START: OnceLock<Instant> = OnceLock::new();

/// Mark the process start. Call first thing in `main`.
pub fn init() {
    let _ = START.set(Instant::now());
}

/// Milliseconds since [`init`].
pub fn since_start_ms() -> u128 {
    START.get().map_or(0, |s| s.elapsed().as_millis())
}

/// A named phase; logs its duration and the time since process start when
/// finished.
pub struct Phase {
    name: &'static str,
    t0: Instant,
}

impl Phase {
    pub fn start(name: &'static str) -> Self {
        Self {
            name,
            t0: Instant::now(),
        }
    }

    pub fn done(self) {
        log::info!(
            "startup: {} took {} ms (at +{} ms)",
            self.name,
            self.t0.elapsed().as_millis(),
            since_start_ms()
        );
    }
}
