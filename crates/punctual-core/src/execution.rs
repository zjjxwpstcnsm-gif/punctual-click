use std::sync::atomic::{AtomicBool, Ordering};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::ClickMode;

/// Relative deadlines used to prepare a task before its target time.
///
/// The defaults intentionally finish all expensive browser work before the
/// exact click deadline. The deadline itself should only dispatch the already
/// resolved click.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionPlanConfig {
    pub prewarm_before_ms: i64,
    pub resolve_before_ms: i64,
    pub arm_before_ms: i64,
}

impl Default for ExecutionPlanConfig {
    fn default() -> Self {
        Self {
            prewarm_before_ms: 60_000,
            resolve_before_ms: 10_000,
            arm_before_ms: 1_000,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionPhase {
    Waiting,
    Prewarming,
    ResolvingTarget,
    Armed,
    Due,
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionPlan {
    pub prewarm_at: DateTime<Utc>,
    pub resolve_at: DateTime<Utc>,
    pub arm_at: DateTime<Utc>,
    pub scheduled_at: DateTime<Utc>,
    pub click_deadline: DateTime<Utc>,
}

impl ExecutionPlan {
    pub fn new(
        scheduled_at: DateTime<Utc>,
        click_mode: &ClickMode,
        config: ExecutionPlanConfig,
    ) -> Result<Self, ExecutionPlanError> {
        config.validate()?;

        let click_deadline = match click_mode {
            ClickMode::Strict => scheduled_at,
            ClickMode::WaitUntilClickable { grace_period_ms } => {
                let grace_period_ms = i64::try_from(*grace_period_ms)
                    .map_err(|_| ExecutionPlanError::GracePeriodTooLarge)?;
                scheduled_at + Duration::milliseconds(grace_period_ms)
            }
        };

        Ok(Self {
            prewarm_at: scheduled_at - Duration::milliseconds(config.prewarm_before_ms),
            resolve_at: scheduled_at - Duration::milliseconds(config.resolve_before_ms),
            arm_at: scheduled_at - Duration::milliseconds(config.arm_before_ms),
            scheduled_at,
            click_deadline,
        })
    }

    pub fn phase_at(&self, now: DateTime<Utc>) -> ExecutionPhase {
        if now < self.prewarm_at {
            ExecutionPhase::Waiting
        } else if now < self.resolve_at {
            ExecutionPhase::Prewarming
        } else if now < self.arm_at {
            ExecutionPhase::ResolvingTarget
        } else if now < self.scheduled_at {
            ExecutionPhase::Armed
        } else if now <= self.click_deadline {
            ExecutionPhase::Due
        } else {
            ExecutionPhase::Expired
        }
    }

    /// Returns the next phase boundary after `now`, if the execution window has
    /// not expired. This lets a scheduler sleep until meaningful work is due
    /// instead of polling continuously.
    pub fn next_transition_after(&self, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
        [
            self.prewarm_at,
            self.resolve_at,
            self.arm_at,
            self.scheduled_at,
            self.click_deadline,
        ]
        .into_iter()
        .find(|deadline| *deadline > now)
    }
}

impl ExecutionPlanConfig {
    fn validate(self) -> Result<(), ExecutionPlanError> {
        if self.prewarm_before_ms < 0 || self.resolve_before_ms < 0 || self.arm_before_ms < 0 {
            return Err(ExecutionPlanError::NegativeOffset);
        }
        if self.prewarm_before_ms < self.resolve_before_ms
            || self.resolve_before_ms < self.arm_before_ms
        {
            return Err(ExecutionPlanError::InvalidOffsetOrder);
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionPlanError {
    #[error("execution-plan offsets cannot be negative")]
    NegativeOffset,
    #[error("execution-plan offsets must satisfy prewarm >= resolve >= arm")]
    InvalidOffsetOrder,
    #[error("click grace period is too large")]
    GracePeriodTooLarge,
}

/// Process-local guard that enforces one logical click attempt per task run.
///
/// Browser retries may re-resolve or re-check a target, but only the worker
/// that successfully claims this guard is allowed to dispatch mouse events.
#[derive(Debug, Default)]
pub struct ClickAttemptGuard {
    claimed: AtomicBool,
}

impl ClickAttemptGuard {
    pub const fn new() -> Self {
        Self {
            claimed: AtomicBool::new(false),
        }
    }

    pub fn try_claim(&self) -> bool {
        self.claimed
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    pub fn is_claimed(&self) -> bool {
        self.claimed.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, thread};

    use chrono::TimeZone;

    use super::*;

    fn scheduled() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 8, 20, 11, 0, 0)
            .single()
            .unwrap()
    }

    #[test]
    fn creates_expected_default_deadlines() {
        let plan = ExecutionPlan::new(
            scheduled(),
            &ClickMode::WaitUntilClickable {
                grace_period_ms: 3_000,
            },
            ExecutionPlanConfig::default(),
        )
        .unwrap();

        assert_eq!(plan.prewarm_at, scheduled() - Duration::seconds(60));
        assert_eq!(plan.resolve_at, scheduled() - Duration::seconds(10));
        assert_eq!(plan.arm_at, scheduled() - Duration::seconds(1));
        assert_eq!(plan.click_deadline, scheduled() + Duration::seconds(3));
    }

    #[test]
    fn maps_time_to_execution_phase() {
        let plan = ExecutionPlan::new(
            scheduled(),
            &ClickMode::Strict,
            ExecutionPlanConfig::default(),
        )
        .unwrap();

        assert_eq!(
            plan.phase_at(scheduled() - Duration::seconds(61)),
            ExecutionPhase::Waiting
        );
        assert_eq!(
            plan.phase_at(scheduled() - Duration::seconds(30)),
            ExecutionPhase::Prewarming
        );
        assert_eq!(
            plan.phase_at(scheduled() - Duration::seconds(5)),
            ExecutionPhase::ResolvingTarget
        );
        assert_eq!(
            plan.phase_at(scheduled() - Duration::milliseconds(500)),
            ExecutionPhase::Armed
        );
        assert_eq!(plan.phase_at(scheduled()), ExecutionPhase::Due);
        assert_eq!(
            plan.phase_at(scheduled() + Duration::milliseconds(1)),
            ExecutionPhase::Expired
        );
    }

    #[test]
    fn rejects_out_of_order_offsets() {
        let result = ExecutionPlan::new(
            scheduled(),
            &ClickMode::Strict,
            ExecutionPlanConfig {
                prewarm_before_ms: 1_000,
                resolve_before_ms: 10_000,
                arm_before_ms: 100,
            },
        );
        assert_eq!(result.unwrap_err(), ExecutionPlanError::InvalidOffsetOrder);
    }

    #[test]
    fn only_one_thread_can_claim_click() {
        let guard = Arc::new(ClickAttemptGuard::new());
        let handles = (0..16)
            .map(|_| {
                let guard = Arc::clone(&guard);
                thread::spawn(move || guard.try_claim())
            })
            .collect::<Vec<_>>();

        let winners = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .filter(|won| *won)
            .count();

        assert_eq!(winners, 1);
        assert!(guard.is_claimed());
    }
}
