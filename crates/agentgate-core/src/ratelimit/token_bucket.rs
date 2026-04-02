use std::time::Instant;

pub struct TokenBucket {
    tokens: f64,
    max_tokens: f64,
    refill_rate: f64,
    last_refill: Instant,
}

impl TokenBucket {
    pub fn new(max_per_minute: u64) -> Self {
        Self::new_with_window(max_per_minute, 60)
    }

    /// Create a bucket with an explicit refill window. Used by the policy engine's
    /// per-rule rate limiters where the window is specified in the policy file.
    pub fn new_with_window(max_calls: u64, window_seconds: u64) -> Self {
        let max = max_calls as f64;
        let window = window_seconds.max(1) as f64;
        Self {
            tokens: max,
            max_tokens: max,
            refill_rate: max / window,
            last_refill: Instant::now(),
        }
    }

    pub fn try_consume(&mut self) -> bool {
        self.refill();
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    pub fn retry_after_secs(&self) -> u64 {
        if self.tokens >= 1.0 {
            return 0;
        }
        ((1.0 - self.tokens) / self.refill_rate).ceil() as u64
    }

    fn refill(&mut self) {
        let elapsed = self.last_refill.elapsed().as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.refill_rate).min(self.max_tokens);
        self.last_refill = Instant::now();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exhausts_then_throttles() {
        let mut b = TokenBucket::new(2);
        assert!(b.try_consume());
        assert!(b.try_consume());
        assert!(!b.try_consume());
    }

    #[test]
    fn retry_after_is_positive_when_empty() {
        let mut b = TokenBucket::new(60);
        for _ in 0..60 {
            b.try_consume();
        }
        assert!(!b.try_consume());
        assert!(b.retry_after_secs() >= 1);
    }
}
