//! Leaky-bucket rate limiter and transmission shaper for convergence layer adapters.

use std::time::Duration;

/// Leaky-bucket rate limiter and transmission shaper.
///
/// Regulates aggregate transmission throughput by accumulating tokens at a steady rate
/// (`rate_bytes_per_sec`) up to a maximum burst headroom (`burst_capacity`).
/// When transmission demand exceeds available tokens, calls to [`acquire`] sleep
/// asynchronously until sufficient capacity is replenished.
#[derive(Debug)]
pub struct LeakyBucket {
    rate_bytes_per_sec: u64,
    burst_capacity: f64,
    tokens: f64,
    last_update: tokio::time::Instant,
}

impl LeakyBucket {
    /// Creates a new leaky bucket rate limiter.
    ///
    /// # Arguments
    /// * `rate_bytes_per_sec`: Average allowed transmission rate in bytes/second. If 0, rate limiting is disabled.
    /// * `burst_capacity`: Maximum accumulated tokens in bytes allowed for immediate burst transmission.
    pub fn new(rate_bytes_per_sec: u64, burst_capacity: usize) -> Self {
        Self {
            rate_bytes_per_sec,
            burst_capacity: burst_capacity as f64,
            tokens: burst_capacity as f64,
            last_update: tokio::time::Instant::now(),
        }
    }

    /// Returns the configured rate limit in bytes per second.
    #[must_use]
    pub fn rate_bytes_per_sec(&self) -> u64 {
        self.rate_bytes_per_sec
    }

    /// Returns the maximum burst capacity in bytes.
    #[must_use]
    pub fn burst_capacity(&self) -> f64 {
        self.burst_capacity
    }

    /// Returns the currently available token count in bytes.
    #[must_use]
    pub fn available_tokens(&self) -> f64 {
        let now = tokio::time::Instant::now();
        let elapsed = now.duration_since(self.last_update).as_secs_f64();
        (self.tokens + elapsed * (self.rate_bytes_per_sec as f64)).min(self.burst_capacity)
    }

    /// Asynchronously acquires tokens for a transmission of `bytes_len` bytes.
    ///
    /// If sufficient tokens are present, they are deducted immediately.
    /// Otherwise, sleeps asynchronously for the deficit time required to replenish tokens.
    pub async fn acquire(&mut self, bytes_len: usize) {
        if self.rate_bytes_per_sec == 0 {
            return;
        }

        let now = tokio::time::Instant::now();
        let elapsed = now.duration_since(self.last_update).as_secs_f64();
        self.tokens =
            (self.tokens + elapsed * (self.rate_bytes_per_sec as f64)).min(self.burst_capacity);
        self.last_update = now;

        let cost = bytes_len as f64;
        if self.tokens < cost {
            let deficit = cost - self.tokens;
            let wait_secs = deficit / (self.rate_bytes_per_sec as f64);
            tokio::time::sleep(Duration::from_secs_f64(wait_secs)).await;
            self.tokens = 0.0;
            self.last_update = tokio::time::Instant::now();
        } else {
            self.tokens -= cost;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_leaky_bucket_burst_instantaneous() {
        let mut bucket = LeakyBucket::new(10_000, 1000); // 10 KB/s, 1000 B burst
        let start = tokio::time::Instant::now();

        // Immediate consumption of initial burst
        bucket.acquire(1000).await;
        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_millis(50),
            "Burst acquisition should be near instantaneous, took {elapsed:?}"
        );
    }

    #[tokio::test]
    async fn test_leaky_bucket_pacing_delay() {
        let mut bucket = LeakyBucket::new(10_000, 500); // 10 KB/s, 500 B burst
        let start = tokio::time::Instant::now();

        // Exhaust initial burst
        bucket.acquire(500).await;

        // Consuming 1000 more bytes at 10,000 B/s requires ~100ms
        bucket.acquire(1000).await;
        let elapsed = start.elapsed();
        assert!(
            elapsed >= Duration::from_millis(80),
            "Paced acquisition should sleep for token replenishment, took {elapsed:?}"
        );
        assert!(
            elapsed < Duration::from_millis(300),
            "Sleep duration should not excessively overshoot"
        );
    }

    #[tokio::test]
    async fn test_leaky_bucket_zero_rate_noop() {
        let mut bucket = LeakyBucket::new(0, 1000);
        let start = tokio::time::Instant::now();
        bucket.acquire(50_000).await;
        assert!(start.elapsed() < Duration::from_millis(20));
    }

    #[tokio::test]
    async fn test_leaky_bucket_token_replenishment_cap() {
        let mut bucket = LeakyBucket::new(10_000, 500);
        bucket.acquire(500).await;
        // Wait 100ms: at 10 KB/s, 1000 tokens generated, but capped at 500
        tokio::time::sleep(Duration::from_millis(100)).await;
        let tokens = bucket.available_tokens();
        assert!(tokens <= 500.0);
        assert!(tokens >= 450.0);
    }
}
