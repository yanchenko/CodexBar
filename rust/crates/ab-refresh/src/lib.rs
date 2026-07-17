//! Fixed refresh intervals + adaptive policy (port of AdaptiveRefreshPolicyCore).
//!
//! Pure functions only — no clock, no ProcessInfo. Callers supply `now` and host signals.

use std::time::{Duration, SystemTime};

/// Allowed fixed interval seconds for `ab_set_refresh_interval_secs` (0 = manual).
pub const ALLOWED_INTERVALS: &[u32] = &[0, 60, 120, 300, 900, 1800];

/// Default fixed interval when not in manual/adaptive (5 minutes).
pub const DEFAULT_INTERVAL_SECS: u32 = 300;

/// Nominal interval used by consumers that need one number in adaptive mode heuristics.
pub const NOMINAL_INTERVAL_SECS: u32 = 300;

/// Returns true if `secs` is in the allowed fixed-interval set.
pub fn is_allowed_interval(secs: u32) -> bool {
    ALLOWED_INTERVALS.contains(&secs)
}

/// Adaptive decision reason (stable wire/log token).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdaptiveReason {
    RecentInteraction,
    Warm,
    Idle,
    LongIdle,
    Constrained,
}

impl AdaptiveReason {
    pub fn as_str(self) -> &'static str {
        match self {
            AdaptiveReason::RecentInteraction => "recentInteraction",
            AdaptiveReason::Warm => "warm",
            AdaptiveReason::Idle => "idle",
            AdaptiveReason::LongIdle => "longIdle",
            AdaptiveReason::Constrained => "constrained",
        }
    }
}

/// Pure adaptive policy input (host supplies signals + last menu open).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdaptiveInput {
    pub now: SystemTime,
    pub last_menu_open_at: Option<SystemTime>,
    pub low_power: bool,
    pub thermal_serious: bool,
}

/// Next delay + reason from the adaptive table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdaptiveDecision {
    pub delay: Duration,
    pub reason: AdaptiveReason,
}

impl AdaptiveDecision {
    pub fn delay_secs(self) -> u64 {
        self.delay.as_secs()
    }
}

// Thresholds / delays mirror Sources/AdaptiveRefreshCore/AdaptiveRefreshPolicyCore.swift
const RECENT_INTERACTION_THRESHOLD: Duration = Duration::from_secs(5 * 60);
const WARM_THRESHOLD: Duration = Duration::from_secs(60 * 60);
const IDLE_THRESHOLD: Duration = Duration::from_secs(4 * 60 * 60);

const RECENT_INTERACTION_DELAY: Duration = Duration::from_secs(2 * 60);
const WARM_DELAY: Duration = Duration::from_secs(5 * 60);
const IDLE_DELAY: Duration = Duration::from_secs(15 * 60);
const LONG_IDLE_DELAY: Duration = Duration::from_secs(30 * 60);
const CONSTRAINED_DELAY: Duration = Duration::from_secs(30 * 60);

/// Compute next adaptive delay. First matching row wins (design table).
pub fn next_adaptive_delay(input: &AdaptiveInput) -> AdaptiveDecision {
    if input.low_power || input.thermal_serious {
        return AdaptiveDecision {
            delay: CONSTRAINED_DELAY,
            reason: AdaptiveReason::Constrained,
        };
    }

    let Some(last) = input.last_menu_open_at else {
        return AdaptiveDecision {
            delay: LONG_IDLE_DELAY,
            reason: AdaptiveReason::LongIdle,
        };
    };

    // Future or clock-adjusted timestamps yield a "negative" age → treat as recent.
    let age = match input.now.duration_since(last) {
        Ok(d) => d,
        Err(_) => Duration::ZERO,
    };

    if age <= RECENT_INTERACTION_THRESHOLD {
        return AdaptiveDecision {
            delay: RECENT_INTERACTION_DELAY,
            reason: AdaptiveReason::RecentInteraction,
        };
    }
    if age <= WARM_THRESHOLD {
        return AdaptiveDecision {
            delay: WARM_DELAY,
            reason: AdaptiveReason::Warm,
        };
    }
    if age < IDLE_THRESHOLD {
        return AdaptiveDecision {
            delay: IDLE_DELAY,
            reason: AdaptiveReason::Idle,
        };
    }
    AdaptiveDecision {
        delay: LONG_IDLE_DELAY,
        reason: AdaptiveReason::LongIdle,
    }
}

/// Resolve the sleep duration for the worker loop.
///
/// - Fixed mode (`adaptive == false`): use `interval_secs` (`0` → manual / no auto sleep).
/// - Adaptive mode: ignore fixed interval for sleep length; use policy table.
pub fn resolve_sleep_secs(
    interval_secs: u32,
    adaptive: bool,
    input: &AdaptiveInput,
) -> Option<(u64, Option<AdaptiveReason>)> {
    if adaptive {
        let d = next_adaptive_delay(input);
        return Some((d.delay_secs(), Some(d.reason)));
    }
    if interval_secs == 0 {
        return None; // manual
    }
    Some((u64::from(interval_secs), None))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t0() -> SystemTime {
        UNIX_EPOCH_PLUS(0)
    }

    #[allow(non_snake_case)]
    fn UNIX_EPOCH_PLUS(secs: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(secs)
    }

    #[test]
    fn allowed_intervals_match_design() {
        assert!(is_allowed_interval(0));
        assert!(is_allowed_interval(60));
        assert!(is_allowed_interval(120));
        assert!(is_allowed_interval(300));
        assert!(is_allowed_interval(900));
        assert!(is_allowed_interval(1800));
        assert!(!is_allowed_interval(7));
        assert!(!is_allowed_interval(1));
        assert!(!is_allowed_interval(600));
    }

    #[test]
    fn constrained_low_power() {
        let d = next_adaptive_delay(&AdaptiveInput {
            now: t0(),
            last_menu_open_at: Some(t0()),
            low_power: true,
            thermal_serious: false,
        });
        assert_eq!(d.reason, AdaptiveReason::Constrained);
        assert_eq!(d.delay, CONSTRAINED_DELAY);
    }

    #[test]
    fn constrained_thermal() {
        let d = next_adaptive_delay(&AdaptiveInput {
            now: t0(),
            last_menu_open_at: Some(t0()),
            low_power: false,
            thermal_serious: true,
        });
        assert_eq!(d.reason, AdaptiveReason::Constrained);
        assert_eq!(d.delay_secs(), 30 * 60);
    }

    #[test]
    fn no_menu_open_is_long_idle() {
        let d = next_adaptive_delay(&AdaptiveInput {
            now: t0(),
            last_menu_open_at: None,
            low_power: false,
            thermal_serious: false,
        });
        assert_eq!(d.reason, AdaptiveReason::LongIdle);
        assert_eq!(d.delay_secs(), 30 * 60);
    }

    #[test]
    fn recent_interaction_within_5m() {
        let now = UNIX_EPOCH_PLUS(1000);
        let last = UNIX_EPOCH_PLUS(1000 - 60); // 1 min ago
        let d = next_adaptive_delay(&AdaptiveInput {
            now,
            last_menu_open_at: Some(last),
            low_power: false,
            thermal_serious: false,
        });
        assert_eq!(d.reason, AdaptiveReason::RecentInteraction);
        assert_eq!(d.delay_secs(), 2 * 60);
    }

    #[test]
    fn future_menu_timestamp_is_recent() {
        let now = UNIX_EPOCH_PLUS(1000);
        let last = UNIX_EPOCH_PLUS(2000); // future
        let d = next_adaptive_delay(&AdaptiveInput {
            now,
            last_menu_open_at: Some(last),
            low_power: false,
            thermal_serious: false,
        });
        assert_eq!(d.reason, AdaptiveReason::RecentInteraction);
    }

    #[test]
    fn warm_between_5m_and_1h() {
        let now = UNIX_EPOCH_PLUS(10_000);
        let last = UNIX_EPOCH_PLUS(10_000 - 20 * 60); // 20 min ago
        let d = next_adaptive_delay(&AdaptiveInput {
            now,
            last_menu_open_at: Some(last),
            low_power: false,
            thermal_serious: false,
        });
        assert_eq!(d.reason, AdaptiveReason::Warm);
        assert_eq!(d.delay_secs(), 5 * 60);
    }

    #[test]
    fn idle_between_1h_and_4h() {
        let now = UNIX_EPOCH_PLUS(20_000);
        let last = UNIX_EPOCH_PLUS(20_000 - 2 * 60 * 60); // 2h ago
        let d = next_adaptive_delay(&AdaptiveInput {
            now,
            last_menu_open_at: Some(last),
            low_power: false,
            thermal_serious: false,
        });
        assert_eq!(d.reason, AdaptiveReason::Idle);
        assert_eq!(d.delay_secs(), 15 * 60);
    }

    #[test]
    fn long_idle_after_4h() {
        let now = UNIX_EPOCH_PLUS(50_000);
        let last = UNIX_EPOCH_PLUS(50_000 - 5 * 60 * 60); // 5h ago
        let d = next_adaptive_delay(&AdaptiveInput {
            now,
            last_menu_open_at: Some(last),
            low_power: false,
            thermal_serious: false,
        });
        assert_eq!(d.reason, AdaptiveReason::LongIdle);
        assert_eq!(d.delay_secs(), 30 * 60);
    }

    #[test]
    fn reason_as_str_stable() {
        assert_eq!(AdaptiveReason::Warm.as_str(), "warm");
        assert_eq!(AdaptiveReason::Constrained.as_str(), "constrained");
    }

    #[test]
    fn resolve_sleep_manual_none() {
        let input = AdaptiveInput {
            now: t0(),
            last_menu_open_at: None,
            low_power: false,
            thermal_serious: false,
        };
        assert!(resolve_sleep_secs(0, false, &input).is_none());
    }

    #[test]
    fn resolve_sleep_fixed() {
        let input = AdaptiveInput {
            now: t0(),
            last_menu_open_at: None,
            low_power: false,
            thermal_serious: false,
        };
        let (secs, reason) = resolve_sleep_secs(120, false, &input).unwrap();
        assert_eq!(secs, 120);
        assert!(reason.is_none());
    }

    #[test]
    fn resolve_sleep_adaptive_uses_policy() {
        let input = AdaptiveInput {
            now: t0(),
            last_menu_open_at: None,
            low_power: false,
            thermal_serious: false,
        };
        let (secs, reason) = resolve_sleep_secs(0, true, &input).unwrap();
        assert_eq!(secs, 30 * 60);
        assert_eq!(reason, Some(AdaptiveReason::LongIdle));
    }
}
