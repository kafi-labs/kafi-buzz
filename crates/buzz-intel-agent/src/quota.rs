//! Per-scope LLM turn quota.
//!
//! This is a **cost** control, not a protocol control. The relay already
//! rate-limits protocol admission (WS connects and EVENT ingest) via
//! `buzz_pubsub::rate_limiter::RedisRateLimiter`, but that says nothing about
//! how many paid LLM turns an agent runs: one admitted mention can cost a full
//! gateway turn. Nothing between the harness and the intel gateway bounded that
//! before this module.
//!
//! It lives in the adapter rather than the relay on purpose — the relay's fork
//! surface stays zero, and the quota sits directly in front of the only code
//! path that spends money.
//!
//! ## Window semantics
//!
//! Fixed window, per scope key: the first turn in a window starts it, and the
//! window resets once `window` has elapsed. Fixed (not sliding) is deliberate —
//! it needs one counter and one timestamp per key, and "allow / deny / reset"
//! is exactly what an operator can reason about from the logs.
//!
//! Elapsed time is measured against a **monotonic** clock supplied by the
//! caller, never a wall clock and never a timestamp taken from an event. Event
//! timestamps are author-supplied and can be skewed by an attacker.

use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Quota limits. `max_turns_per_window == None` disables enforcement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuotaConfig {
    /// Maximum gateway turns per window per scope. `None` disables the quota.
    pub max_turns_per_window: Option<u32>,
    /// Window length.
    pub window: Duration,
}

impl QuotaConfig {
    /// Build a config, treating `0` turns as "disabled" rather than "deny all".
    ///
    /// Deny-all would be a foot-gun: an operator setting `0` to mean "off" would
    /// silently take every agent offline, and the failure looks like a gateway
    /// outage rather than a config mistake.
    pub fn new(max_turns_per_window: u32, window_secs: u64) -> Self {
        Self {
            max_turns_per_window: (max_turns_per_window > 0).then_some(max_turns_per_window),
            window: Duration::from_secs(window_secs.max(1)),
        }
    }

    /// Whether enforcement is active.
    pub fn is_enabled(&self) -> bool {
        self.max_turns_per_window.is_some()
    }
}

/// Outcome of a quota check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuotaDecision {
    /// Turn is allowed; the turn has been counted.
    Allow {
        /// Turns still available in the current window after this one.
        remaining: u32,
    },
    /// Turn is refused; nothing was counted and no gateway call must be made.
    Deny {
        /// The configured limit that was hit.
        limit: u32,
        /// Seconds until the current window resets.
        retry_after_secs: u64,
    },
}

#[derive(Debug, Clone, Copy)]
struct Window {
    started_at: Instant,
    count: u32,
}

/// Fixed-window turn counter keyed by an opaque scope string.
///
/// Not `Sync` on its own — the adapter holds it behind the same mutex
/// discipline as its other per-process state.
#[derive(Debug)]
pub struct TurnQuota {
    cfg: QuotaConfig,
    windows: HashMap<String, Window>,
}

impl TurnQuota {
    /// Create a quota tracker.
    pub fn new(cfg: QuotaConfig) -> Self {
        Self {
            cfg,
            windows: HashMap::new(),
        }
    }

    /// Configured limits.
    pub fn config(&self) -> QuotaConfig {
        self.cfg
    }

    /// Check the quota for `key` and, when allowed, count the turn.
    ///
    /// `now` must come from a monotonic source. Callers in the adapter pass
    /// `Instant::now()`; tests pass a base instant plus offsets so window reset
    /// is exercised without sleeping.
    ///
    /// Counting happens on the allow path only: a denied turn never reaches the
    /// gateway, so charging it would make the window heal more slowly than the
    /// spend it is meant to track.
    pub fn check_and_record_at(&mut self, key: &str, now: Instant) -> QuotaDecision {
        let Some(limit) = self.cfg.max_turns_per_window else {
            return QuotaDecision::Allow {
                remaining: u32::MAX,
            };
        };

        // Admission pays an O(n) scan over tracked scopes. The map is bounded by
        // active community/channel/agent scopes, and a large map is exactly when
        // reclaiming elapsed scopes is worth the scan and immediately shrinks n.
        self.evict_expired_at(now);

        let window = self.cfg.window;
        let entry = self.windows.entry(key.to_owned()).or_insert(Window {
            started_at: now,
            count: 0,
        });

        // Reset first, so a caller returning after a long idle is not judged
        // against a stale window.
        if now.duration_since(entry.started_at) >= window {
            entry.started_at = now;
            entry.count = 0;
        }

        if entry.count >= limit {
            let elapsed = now.duration_since(entry.started_at);
            let retry_after = window.saturating_sub(elapsed);
            return QuotaDecision::Deny {
                limit,
                // Round up so "retry in 0s" is never reported for a live window.
                retry_after_secs: retry_after.as_secs().max(1),
            };
        }

        entry.count += 1;
        QuotaDecision::Allow {
            remaining: limit.saturating_sub(entry.count),
        }
    }

    /// Convenience wrapper using the process monotonic clock.
    pub fn check_and_record(&mut self, key: &str) -> QuotaDecision {
        self.check_and_record_at(key, Instant::now())
    }

    /// Number of scope keys currently tracked (for diagnostics and tests).
    pub fn tracked_scopes(&self) -> usize {
        self.windows.len()
    }

    /// Drop windows that have fully elapsed, bounding memory for installations
    /// with many short-lived channels.
    pub fn evict_expired_at(&mut self, now: Instant) {
        let window = self.cfg.window;
        self.windows
            .retain(|_, w| now.duration_since(w.started_at) < window);
    }
}

/// Build the quota scope key.
///
/// Deliberately contains **no secrets**: a community host, a channel UUID (or an
/// ACP session id when the prompt carried no channel), and the configured intel
/// agent name. All three are safe to log, which matters because a denied turn is
/// logged with its key.
pub fn scope_key(community: Option<&str>, channel: Option<&str>, agent: &str) -> String {
    let community = community.unwrap_or("local");
    let channel = channel.unwrap_or("nochannel");
    let agent = if agent.is_empty() { "noagent" } else { agent };
    format!("{community}/{channel}/{agent}")
}

/// Owner-visible message posted to the channel when a turn is refused.
///
/// States the limit and when it resets. An agent that silently stops answering
/// is indistinguishable from one that is broken, so the refusal has to be
/// legible to the person in the channel.
pub fn quota_exceeded_message(limit: u32, window_secs: u64, retry_after_secs: u64) -> String {
    format!(
        "⏳ Turn quota reached ({limit} turns per {window_secs}s for this channel). \
         Try again in about {retry_after_secs}s."
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(max: u32, secs: u64) -> QuotaConfig {
        QuotaConfig::new(max, secs)
    }

    #[test]
    fn allows_up_to_the_limit_then_denies() {
        let mut q = TurnQuota::new(cfg(3, 60));
        let t0 = Instant::now();

        assert_eq!(
            q.check_and_record_at("k", t0),
            QuotaDecision::Allow { remaining: 2 }
        );
        assert_eq!(
            q.check_and_record_at("k", t0),
            QuotaDecision::Allow { remaining: 1 }
        );
        assert_eq!(
            q.check_and_record_at("k", t0),
            QuotaDecision::Allow { remaining: 0 }
        );

        match q.check_and_record_at("k", t0) {
            QuotaDecision::Deny {
                limit,
                retry_after_secs,
            } => {
                assert_eq!(limit, 3);
                assert!(retry_after_secs > 0 && retry_after_secs <= 60);
            }
            other => panic!("expected Deny, got {other:?}"),
        }
    }

    #[test]
    fn window_resets_after_it_elapses() {
        let mut q = TurnQuota::new(cfg(1, 60));
        let t0 = Instant::now();

        assert!(matches!(
            q.check_and_record_at("k", t0),
            QuotaDecision::Allow { .. }
        ));
        assert!(matches!(
            q.check_and_record_at("k", t0 + Duration::from_secs(59)),
            QuotaDecision::Deny { .. }
        ));
        // Exactly at the boundary the window is over.
        assert!(matches!(
            q.check_and_record_at("k", t0 + Duration::from_secs(60)),
            QuotaDecision::Allow { .. }
        ));
    }

    #[test]
    fn scopes_are_independent() {
        let mut q = TurnQuota::new(cfg(1, 60));
        let t0 = Instant::now();

        assert!(matches!(
            q.check_and_record_at("a", t0),
            QuotaDecision::Allow { .. }
        ));
        // A different scope has its own budget.
        assert!(matches!(
            q.check_and_record_at("b", t0),
            QuotaDecision::Allow { .. }
        ));
        assert!(matches!(
            q.check_and_record_at("a", t0),
            QuotaDecision::Deny { .. }
        ));
    }

    #[test]
    fn a_denied_turn_is_not_counted() {
        let mut q = TurnQuota::new(cfg(1, 60));
        let t0 = Instant::now();

        q.check_and_record_at("k", t0);
        // Several denials inside the window...
        for _ in 0..5 {
            assert!(matches!(
                q.check_and_record_at("k", t0 + Duration::from_secs(1)),
                QuotaDecision::Deny { .. }
            ));
        }
        // ...do not extend or re-arm the window; it still resets on schedule.
        assert!(matches!(
            q.check_and_record_at("k", t0 + Duration::from_secs(60)),
            QuotaDecision::Allow { .. }
        ));
    }

    #[test]
    fn zero_means_disabled_not_deny_all() {
        let c = cfg(0, 60);
        assert!(!c.is_enabled());
        let mut q = TurnQuota::new(c);
        let t0 = Instant::now();
        for _ in 0..100 {
            assert!(matches!(
                q.check_and_record_at("k", t0),
                QuotaDecision::Allow { .. }
            ));
        }
    }

    #[test]
    fn retry_after_shrinks_as_the_window_drains() {
        let mut q = TurnQuota::new(cfg(1, 100));
        let t0 = Instant::now();
        q.check_and_record_at("k", t0);

        let early = match q.check_and_record_at("k", t0 + Duration::from_secs(10)) {
            QuotaDecision::Deny {
                retry_after_secs, ..
            } => retry_after_secs,
            other => panic!("expected Deny, got {other:?}"),
        };
        let late = match q.check_and_record_at("k", t0 + Duration::from_secs(90)) {
            QuotaDecision::Deny {
                retry_after_secs, ..
            } => retry_after_secs,
            other => panic!("expected Deny, got {other:?}"),
        };
        assert!(late < early, "retry_after must shrink: {late} !< {early}");
    }

    #[test]
    fn window_secs_is_clamped_above_zero() {
        // A zero-length window would reset on every check and never enforce.
        let c = QuotaConfig::new(1, 0);
        assert!(c.window >= Duration::from_secs(1));
    }

    #[test]
    fn expired_windows_are_evicted() {
        let mut q = TurnQuota::new(cfg(5, 60));
        let t0 = Instant::now();
        q.check_and_record_at("a", t0);
        q.check_and_record_at("b", t0);
        assert_eq!(q.tracked_scopes(), 2);

        q.evict_expired_at(t0 + Duration::from_secs(30));
        assert_eq!(q.tracked_scopes(), 2, "live windows must survive eviction");

        q.evict_expired_at(t0 + Duration::from_secs(61));
        assert_eq!(q.tracked_scopes(), 0);
    }

    #[test]
    fn enforcement_evicts_expired_scopes_via_check_and_record() {
        let mut q = TurnQuota::new(cfg(5, 60));
        let t0 = Instant::now();
        q.check_and_record_at("expired-a", t0);
        q.check_and_record_at("expired-b", t0);
        assert_eq!(q.tracked_scopes(), 2);

        assert!(matches!(
            q.check_and_record_at("current", t0 + Duration::from_secs(61)),
            QuotaDecision::Allow { .. }
        ));
        assert_eq!(
            q.tracked_scopes(),
            1,
            "normal enforcement must evict elapsed scopes before adding the current one"
        );
    }

    #[test]
    fn scope_key_is_stable_and_carries_no_secrets() {
        let k = scope_key(
            Some("relay.example.com"),
            Some("b6b6fab0-1e90-46e0-9540-7a445124dd59"),
            "buzz-cfo-agent",
        );
        assert_eq!(
            k,
            "relay.example.com/b6b6fab0-1e90-46e0-9540-7a445124dd59/buzz-cfo-agent"
        );
        // Same inputs, same key — the counter must not drift per process.
        assert_eq!(
            k,
            scope_key(
                Some("relay.example.com"),
                Some("b6b6fab0-1e90-46e0-9540-7a445124dd59"),
                "buzz-cfo-agent"
            )
        );
    }

    #[test]
    fn scope_key_falls_back_without_panicking() {
        assert_eq!(scope_key(None, None, ""), "local/nochannel/noagent");
    }

    #[test]
    fn quota_message_states_limit_and_retry() {
        let m = quota_exceeded_message(30, 3600, 120);
        assert!(m.contains("30"));
        assert!(m.contains("3600"));
        assert!(m.contains("120"));
    }
}
