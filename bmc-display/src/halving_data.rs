// Copyright (C) 2025  Braiins Systems s.r.o.

/// Blocks between each halving (210,000 blocks ≈ 4 years)
pub const HALVING_INTERVAL: u32 = 210_000;

/// Average block time in seconds (Bitcoin targets ~10 minutes)
pub const AVG_BLOCK_TIME_SECS: u32 = 600;

/// Calculate the next halving block height from current height
#[must_use]
#[expect(clippy::integer_division)] // Intentional: block heights require floor division
pub fn next_halving_block(current_height: u32) -> u32 {
    ((current_height / HALVING_INTERVAL) + 1) * HALVING_INTERVAL
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
}
