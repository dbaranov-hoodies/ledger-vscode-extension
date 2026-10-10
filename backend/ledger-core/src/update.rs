//! When to ask the registry whether the Docker image has a newer version.
//!
//! Asking costs a network round trip that can be slow or fail, and an update replaces
//! containers, so asking is throttled. Time is passed in: nothing here reads a clock.

/// What is known about the local image compared with the registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    UpToDate,
    Outdated,
    /// Never checked, or the registry could not be reached. Never a reason to update.
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LastCheck {
    /// Unix time in seconds.
    pub at: u64,
    pub freshness: Freshness,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckPolicy {
    /// Seconds to wait after an answer before asking again.
    pub interval: u64,
    /// Seconds to wait before asking again after the registry could not be reached.
    pub retry_after_failure: u64,
}

impl Default for CheckPolicy {
    fn default() -> Self {
        Self {
            interval: 6 * 60 * 60,
            retry_after_failure: 60 * 60,
        }
    }
}

/// Whether it is time to ask the registry again.
pub fn is_check_due(now: u64, last: Option<LastCheck>, policy: &CheckPolicy) -> bool {
    let Some(last) = last else {
        return true;
    };
    if now < last.at {
        return true;
    }
    let wait = match last.freshness {
        Freshness::Unknown => policy.retry_after_failure,
        Freshness::UpToDate | Freshness::Outdated => policy.interval,
    };
    now - last.at >= wait
}

#[cfg(test)]
mod tests {
    use super::*;

    const POLICY: CheckPolicy = CheckPolicy {
        interval: 100,
        retry_after_failure: 10,
    };

    fn last(at: u64, freshness: Freshness) -> Option<LastCheck> {
        Some(LastCheck { at, freshness })
    }

    #[test]
    fn a_first_check_is_always_due() {
        assert!(is_check_due(0, None, &POLICY));
    }

    #[test]
    fn an_answer_is_trusted_for_the_whole_interval() {
        for freshness in [Freshness::UpToDate, Freshness::Outdated] {
            assert!(!is_check_due(1_099, last(1_000, freshness), &POLICY));
            assert!(is_check_due(1_100, last(1_000, freshness), &POLICY));
        }
    }

    #[test]
    fn an_unreachable_registry_is_asked_again_sooner_but_not_at_once() {
        let unknown = last(1_000, Freshness::Unknown);

        assert!(!is_check_due(1_009, unknown, &POLICY));
        assert!(is_check_due(1_010, unknown, &POLICY));
    }

    #[test]
    fn a_clock_that_went_backwards_does_not_block_checks() {
        assert!(is_check_due(500, last(1_000, Freshness::UpToDate), &POLICY));
    }

    #[test]
    fn the_default_policy_is_gentle() {
        let policy = CheckPolicy::default();

        assert_eq!(policy.interval, 21_600);
        assert_eq!(policy.retry_after_failure, 3_600);
    }
}
