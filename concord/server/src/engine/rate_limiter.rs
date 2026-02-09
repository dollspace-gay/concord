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
}
