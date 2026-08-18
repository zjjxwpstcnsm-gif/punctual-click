use std::time::{Duration as StdDuration, Instant};

use chrono::{DateTime, Utc};

use crate::utc_now_millis;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreciseTimerConfig {
    /// The final window is waited on a blocking worker with a short spin loop.
    pub spin_window_ms: u64,
}

impl Default for PreciseTimerConfig {
    fn default() -> Self {
        Self { spin_window_ms: 4 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DispatchTiming {
    pub scheduled_at: DateTime<Utc>,
    pub dispatched_at: DateTime<Utc>,
    pub drift_ms: i64,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct PreciseTimer {
    config: PreciseTimerConfig,
}

impl PreciseTimer {
    pub const fn new(config: PreciseTimerConfig) -> Self {
        Self { config }
    }

    /// Waits until a UTC deadline.
    ///
    /// This improves ordinary desktop scheduling jitter, but it is deliberately
    /// not described as hard real-time. Browser and operating-system latency are
    /// still observable and must be recorded by the caller.
    pub async fn wait_until(&self, scheduled_at: DateTime<Utc>) -> DispatchTiming {
        let now = Utc::now();
        let remaining_ms = (scheduled_at - now).num_milliseconds();

        if remaining_ms > 0 {
            let spin_ms = self.config.spin_window_ms.min(remaining_ms as u64);
            let sleep_ms = remaining_ms as u64 - spin_ms;
            if sleep_ms > 0 {
                tokio::time::sleep(StdDuration::from_millis(sleep_ms)).await;
            }

            if spin_ms > 0 {
                let target = scheduled_at;
                let _ = tokio::task::spawn_blocking(move || {
                    let remaining = (target - Utc::now())
                        .to_std()
                        .unwrap_or_else(|_| StdDuration::ZERO);
                    let deadline = Instant::now() + remaining;
                    while Instant::now() < deadline {
                        std::hint::spin_loop();
                    }
                })
                .await;
            }
        }

        let dispatched_at = utc_now_millis();
        DispatchTiming {
            scheduled_at,
            dispatched_at,
            drift_ms: (dispatched_at - scheduled_at).num_milliseconds(),
        }
    }
}
