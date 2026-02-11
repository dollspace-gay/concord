use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Simple token-bucket rate limiter keyed by string (nickname, IP, etc.).
pub struct RateLimiter {
    buckets: Mutex<HashMap<String, Bucket>>,
    max_tokens: u32,
    refill_rate: f64, // tokens per second
}

struct Bucket {
    tokens: f64,
    last_refill: Instant,
}

impl RateLimiter {
    /// Create a rate limiter.
    /// - `max_tokens`: burst capacity
    /// - `per_seconds`: refill one token every N seconds
    pub fn new(max_tokens: u32, per_seconds: f64) -> Self {
        Self {
            buckets: Mutex::new(HashMap::new()),
            max_tokens,
            refill_rate: 1.0 / per_seconds,
        }
    }

    /// Check if an action is allowed for the given key. Returns true if allowed.
    pub fn check(&self, key: &str) -> bool {
        let mut buckets = self.buckets.lock().unwrap();
        let now = Instant::now();

        let bucket = buckets.entry(key.to_string()).or_insert(Bucket {
            tokens: self.max_tokens as f64,
            last_refill: now,
        });

        // Refill tokens based on elapsed time
        let elapsed = now.duration_since(bucket.last_refill).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * self.refill_rate).min(self.max_tokens as f64);
        bucket.last_refill = now;

        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    /// Remove stale entries older than the given duration.
    pub fn cleanup(&self, older_than: Duration) {
        let mut buckets = self.buckets.lock().unwrap();
        let cutoff = Instant::now() - older_than;
        buckets.retain(|_, b| b.last_refill > cutoff);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allows_burst() {
        let limiter = RateLimiter::new(5, 1.0);
        for _ in 0..5 {
            assert!(limiter.check("user1"));
        }
        // 6th should be denied
        assert!(!limiter.check("user1"));
    }

    #[test]
    fn test_different_keys_independent() {
        let limiter = RateLimiter::new(2, 1.0);
        assert!(limiter.check("a"));
        assert!(limiter.check("a"));
        assert!(!limiter.check("a"));
        // Different key should still have tokens
        assert!(limiter.check("b"));
    }

    #[test]
    fn test_cleanup() {
        let limiter = RateLimiter::new(5, 1.0);
        limiter.check("old");
        limiter.cleanup(Duration::from_secs(0));
        // After cleanup with 0 duration, entry should be removed
        let buckets = limiter.buckets.lock().unwrap();
        assert!(buckets.is_empty());
    }

    // ────────────────────────────────────────────────────────────────
    // Additional rate limiter tests
    // ────────────────────────────────────────────────────────────────

    #[test]
    fn test_exactly_at_the_limit() {
        let limiter = RateLimiter::new(3, 1.0);
        // Use exactly 3 tokens (the burst capacity)
        assert!(limiter.check("user"));
        assert!(limiter.check("user"));
        assert!(limiter.check("user"));
        // Now at exactly the limit — next should fail
        assert!(!limiter.check("user"));
    }

    #[test]
    fn test_single_token_bucket() {
        let limiter = RateLimiter::new(1, 1.0);
        assert!(limiter.check("user"));
        assert!(!limiter.check("user"));
    }

    #[test]
    fn test_rate_limit_refill_over_time() {
        let limiter = RateLimiter::new(2, 1.0);
        assert!(limiter.check("user"));
        assert!(limiter.check("user"));
        assert!(!limiter.check("user"));

        // Simulate time passing by directly manipulating the bucket
        {
            let mut buckets = limiter.buckets.lock().unwrap();
            let bucket = buckets.get_mut("user").unwrap();
            // Set last_refill to 2 seconds ago so 2 tokens refill
            bucket.last_refill = Instant::now() - Duration::from_secs(2);
        }

        // After "2 seconds", tokens should have refilled
        assert!(limiter.check("user"));
    }

    #[test]
    fn test_refill_does_not_exceed_max() {
        let limiter = RateLimiter::new(3, 1.0);
        // Use one token
        assert!(limiter.check("user"));

        // Simulate a long time passing (100 seconds)
        {
            let mut buckets = limiter.buckets.lock().unwrap();
            let bucket = buckets.get_mut("user").unwrap();
            bucket.last_refill = Instant::now() - Duration::from_secs(100);
        }

        // Should be capped at max_tokens (3), so 3 checks pass, 4th fails
        assert!(limiter.check("user"));
        assert!(limiter.check("user"));
        assert!(limiter.check("user"));
        assert!(!limiter.check("user"));
    }

    #[test]
    fn test_many_keys_independent() {
        let limiter = RateLimiter::new(1, 1.0);
        // Each key gets its own bucket
        for i in 0..100 {
            let key = format!("user_{i}");
            assert!(limiter.check(&key), "user_{i} should succeed");
        }
        // All should now be exhausted
        for i in 0..100 {
            let key = format!("user_{i}");
            assert!(!limiter.check(&key), "user_{i} should fail on second try");
        }
    }

    #[test]
    fn test_cleanup_preserves_recent_entries() {
        let limiter = RateLimiter::new(5, 1.0);
        limiter.check("recent");
        // Cleanup with a generous duration — should keep the entry
        limiter.cleanup(Duration::from_secs(60));
        let buckets = limiter.buckets.lock().unwrap();
        assert!(buckets.contains_key("recent"));
    }

    #[test]
    fn test_different_refill_rates() {
        // Slower refill rate: 1 token every 2 seconds
        let limiter = RateLimiter::new(2, 2.0);
        assert!(limiter.check("user"));
        assert!(limiter.check("user"));
        assert!(!limiter.check("user"));

        // After 2 seconds, only 1 token should be refilled (rate = 1/2 tokens per second)
        {
            let mut buckets = limiter.buckets.lock().unwrap();
            let bucket = buckets.get_mut("user").unwrap();
            bucket.last_refill = Instant::now() - Duration::from_secs(2);
        }
        assert!(limiter.check("user"));
        assert!(!limiter.check("user"));
    }

    #[test]
    fn test_rapid_burst_all_same_instant() {
        // Simulate rapid burst — all calls happen "instantly"
        let limiter = RateLimiter::new(5, 1.0);
        let mut successes = 0;
        for _ in 0..10 {
            if limiter.check("burst-user") {
                successes += 1;
            }
        }
        assert_eq!(successes, 5, "Should allow exactly max_tokens successes");
    }

    #[test]
    fn test_zero_tokens_after_exhaust() {
        let limiter = RateLimiter::new(3, 1.0);
        limiter.check("user");
        limiter.check("user");
        limiter.check("user");
        // Multiple failures should not go negative or cause issues
        assert!(!limiter.check("user"));
        assert!(!limiter.check("user"));
        assert!(!limiter.check("user"));
    }
}
