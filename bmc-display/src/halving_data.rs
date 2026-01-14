// Copyright (C) 2025  Braiins Systems s.r.o.

/// Blocks between each halving (210,000 blocks ≈ 4 years)
pub const HALVING_INTERVAL: u32 = 210_000;

/// Average block time in seconds (Bitcoin targets ~10 minutes)
pub const AVG_BLOCK_TIME_SECS: u64 = 600;

/// Calculate the next halving block height from current height
#[must_use]
#[expect(clippy::integer_division)] // Intentional: block heights require floor division
pub fn next_halving_block(current_height: u32) -> u32 {
    ((current_height / HALVING_INTERVAL) + 1) * HALVING_INTERVAL
}

/// Represents the countdown state for halving
#[derive(Debug, Clone, Default)]
pub struct HalvingCountdown {
    pub total_seconds: u64,
    pub blocks_remaining: u32,
}

impl HalvingCountdown {
    /// Calculate countdown from current block height
    #[must_use]
    pub fn from_block_height(current_height: u32) -> Self {
        let next_halving = next_halving_block(current_height);
        let blocks_remaining = next_halving.saturating_sub(current_height);
        let total_seconds = u64::from(blocks_remaining) * AVG_BLOCK_TIME_SECS;

        Self {
            total_seconds,
            blocks_remaining,
        }
    }

    /// Decrement by one second
    pub fn tick(&mut self) {
        self.total_seconds = self.total_seconds.saturating_sub(1);
    }

    /// Check if countdown has reached zero
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.total_seconds == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_next_halving_block() {
        // Before first halving
        assert_eq!(next_halving_block(100_000), 210_000);
        // After first halving
        assert_eq!(next_halving_block(210_001), 420_000);
        // After fourth halving (current era)
        assert_eq!(next_halving_block(840_001), 1_050_000);
        // Exactly at halving
        assert_eq!(next_halving_block(840_000), 1_050_000);
    }

    #[test]
    fn test_countdown_from_height() {
        let countdown = HalvingCountdown::from_block_height(1_000_000);

        assert_eq!(countdown.blocks_remaining, 50_000);
        // 50,000 blocks * 600 seconds = 30,000,000 seconds
        assert_eq!(countdown.total_seconds, 30_000_000);
    }

    #[test]
    fn test_tick() {
        let mut countdown = HalvingCountdown {
            total_seconds: 100,
            blocks_remaining: 1,
        };

        countdown.tick();
        assert_eq!(countdown.total_seconds, 99);
        assert!(!countdown.is_complete());
    }

    #[test]
    fn test_tick_at_zero() {
        let mut countdown = HalvingCountdown {
            total_seconds: 0,
            blocks_remaining: 0,
        };

        countdown.tick(); // Should not underflow
        assert_eq!(countdown.total_seconds, 0);
        assert!(countdown.is_complete());
    }
}
