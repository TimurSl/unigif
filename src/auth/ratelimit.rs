//! In-memory limiters: a token bucket per (credential, class) for API
//! requests, and an exponential back-off for failed logins per client IP.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decision {
    pub allowed: bool,
    pub limit: u32,
    pub remaining: u32,
    /// Seconds until the bucket is full again.
    pub reset_secs: u64,
    /// Seconds until one more request is allowed (0 when allowed).
    pub retry_after: u64,
}

struct Bucket {
    tokens: f64,
    last: Instant,
}

#[derive(Default)]
pub struct RateLimiter {
    buckets: Mutex<HashMap<(String, &'static str), Bucket>>,
}

impl RateLimiter {
    pub fn check(&self, who: &str, class: &'static str, per_min: u32) -> Decision {
        self.check_at(who, class, per_min, Instant::now())
    }

    pub fn check_at(&self, who: &str, class: &'static str, per_min: u32, now: Instant) -> Decision {
        let cap = per_min.max(1) as f64;
        let rate = cap / 60.0; // tokens per second
        let mut map = self.buckets.lock().unwrap_or_else(|e| e.into_inner());
        if map.len() > 10_000 {
            map.retain(|_, b| now.duration_since(b.last) < Duration::from_secs(600));
        }
        let b = map.entry((who.to_string(), class)).or_insert(Bucket { tokens: cap, last: now });
        let elapsed = now.saturating_duration_since(b.last).as_secs_f64();
        b.tokens = (b.tokens + elapsed * rate).min(cap);
        b.last = now;
        let allowed = b.tokens >= 1.0;
        if allowed {
            b.tokens -= 1.0;
        }
        Decision {
            allowed,
            limit: per_min,
            remaining: b.tokens.floor() as u32,
            reset_secs: ((cap - b.tokens) / rate).ceil() as u64,
            retry_after: if allowed { 0 } else { ((1.0 - b.tokens) / rate).ceil().max(1.0) as u64 },
        }
    }
}

const MAX_FAILURES: u32 = 5;
const WINDOW: Duration = Duration::from_secs(60);
const MAX_BLOCK: Duration = Duration::from_secs(3600);

#[derive(Default)]
struct Attempts {
    window_start: Option<Instant>,
    failures: u32,
    strikes: u32,
    blocked_until: Option<Instant>,
}

/// Five failed logins a minute per IP, then a lock-out that doubles each time
/// it is earned (1 min, 2 min, 4 min, … capped at an hour). A success clears it.
#[derive(Default)]
pub struct LoginThrottle {
    by_ip: Mutex<HashMap<String, Attempts>>,
}

impl LoginThrottle {
    /// `Err(seconds)` while the client is locked out.
    pub fn check(&self, ip: &str) -> Result<(), u64> {
        self.check_at(ip, Instant::now())
    }

    pub fn check_at(&self, ip: &str, now: Instant) -> Result<(), u64> {
        let map = self.by_ip.lock().unwrap_or_else(|e| e.into_inner());
        match map.get(ip).and_then(|a| a.blocked_until) {
            Some(until) if until > now => Err((until - now).as_secs().max(1)),
            _ => Ok(()),
        }
    }

    pub fn failure(&self, ip: &str) {
        self.failure_at(ip, Instant::now())
    }

    pub fn failure_at(&self, ip: &str, now: Instant) {
        let mut map = self.by_ip.lock().unwrap_or_else(|e| e.into_inner());
        if map.len() > 10_000 {
            map.retain(|_, a| {
                a.blocked_until.is_some_and(|u| u > now)
                    || a.window_start.is_some_and(|w| now.duration_since(w) < MAX_BLOCK)
            });
        }
        let a = map.entry(ip.to_string()).or_default();
        if a.window_start.map_or(true, |w| now.duration_since(w) > WINDOW) {
            a.window_start = Some(now);
            a.failures = 0;
            // Long quiet spell: forgive earlier lock-outs.
            if a.blocked_until.map_or(true, |u| now.duration_since(u) > MAX_BLOCK) {
                a.strikes = 0;
            }
        }
        a.failures += 1;
        if a.failures >= MAX_FAILURES {
            let secs = 60u64.saturating_mul(1u64 << a.strikes.min(10));
            a.blocked_until = Some(now + Duration::from_secs(secs).min(MAX_BLOCK));
            a.strikes += 1;
            a.failures = 0;
            a.window_start = Some(now);
        }
    }

    pub fn success(&self, ip: &str) {
        self.by_ip.lock().unwrap_or_else(|e| e.into_inner()).remove(ip);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_allows_burst_then_refills() {
        let rl = RateLimiter::default();
        let t0 = Instant::now();
        for i in 0..6 {
            let d = rl.check_at("k", "read", 6, t0);
            assert!(d.allowed, "request {i}");
            assert_eq!(d.remaining, 5 - i);
        }
        let d = rl.check_at("k", "read", 6, t0);
        assert!(!d.allowed);
        assert_eq!(d.retry_after, 10); // 6/min = one token per 10 s
        assert!(rl.check_at("k", "read", 6, t0 + Duration::from_secs(10)).allowed);
        assert!(!rl.check_at("k", "read", 6, t0 + Duration::from_secs(10)).allowed);
    }

    #[test]
    fn buckets_are_per_credential_and_class() {
        let rl = RateLimiter::default();
        let t0 = Instant::now();
        assert!(rl.check_at("a", "read", 1, t0).allowed);
        assert!(!rl.check_at("a", "read", 1, t0).allowed);
        assert!(rl.check_at("b", "read", 1, t0).allowed);
        assert!(rl.check_at("a", "upload", 1, t0).allowed);
    }

    #[test]
    fn login_lockout_doubles() {
        let t = LoginThrottle::default();
        let t0 = Instant::now();
        for _ in 0..4 {
            t.failure_at("1.2.3.4", t0);
            assert!(t.check_at("1.2.3.4", t0).is_ok());
        }
        t.failure_at("1.2.3.4", t0);
        assert_eq!(t.check_at("1.2.3.4", t0), Err(60));
        assert!(t.check_at("5.6.7.8", t0).is_ok());
        // After the first lock-out, five more failures earn a longer one.
        let t1 = t0 + Duration::from_secs(61);
        assert!(t.check_at("1.2.3.4", t1).is_ok());
        for _ in 0..5 {
            t.failure_at("1.2.3.4", t1);
        }
        assert_eq!(t.check_at("1.2.3.4", t1), Err(120));
        t.success("1.2.3.4");
        assert!(t.check_at("1.2.3.4", t1).is_ok());
    }

    #[test]
    fn slow_failures_do_not_lock_out() {
        let t = LoginThrottle::default();
        let t0 = Instant::now();
        for i in 0..20 {
            t.failure_at("ip", t0 + Duration::from_secs(i * 61));
            assert!(t.check_at("ip", t0 + Duration::from_secs(i * 61)).is_ok());
        }
    }
}
