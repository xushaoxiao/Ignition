//! Play: server-authoritative outcome generation and prize-pool deduction.
//!
//! **Outcomes are 100% server-side; the frontend only animates.** Not anti-cheat pedantry — the
//! wheel result directly drives prize-pool cost and whether a claim code (and thus billable
//! conversion) can be issued. Client-side outcomes hand the prize pool and invoice to the user.

pub mod play;

use rand::Rng;

/// One prize in the draw pool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prize {
    pub id: i64,
    pub label: String,
    pub weight: i64,
    /// Remaining stock. Zero-stock prizes are excluded from draws.
    pub remaining: i64,
}

impl Prize {
    /// Whether this prize can be drawn right now.
    fn available(&self) -> bool {
        self.weight > 0 && self.remaining > 0
    }
}

/// Draw one prize by weight; returns its index in `prizes`.
///
/// `roll` is injected by callers rather than drawn inside — same rationale as injecting `now` in
/// `telegram::verify`: probability logic must be testable deterministically; "can weight-0 prizes
/// win?" should not require ten thousand random runs.
///
/// `roll` is in `[0, total_weight)` where `total_weight` sums weights of **in-stock** prizes only.
/// Zero-stock prizes are excluded when computing total weight, so we never "draw out of stock then
/// back off".
pub fn draw_with(prizes: &[Prize], roll: i64) -> Option<usize> {
    let total = total_weight(prizes);
    if total == 0 {
        return None;
    }

    let mut cursor = roll.rem_euclid(total);
    for (i, p) in prizes.iter().enumerate() {
        if !p.available() {
            continue;
        }
        if cursor < p.weight {
            return Some(i);
        }
        cursor -= p.weight;
    }
    // Accumulation guarantees a hit when cursor < total; unreachable.
    None
}

/// Sum of weights for in-stock prizes.
pub fn total_weight(prizes: &[Prize]) -> i64 {
    prizes
        .iter()
        .filter(|p| p.available())
        .map(|p| p.weight)
        .sum()
}

/// Draw using a cryptographically secure random source.
///
/// CSPRNG, not plain PRNG: prizes map to real cost; predictable sequences let someone time limited
/// drops.
pub fn draw(prizes: &[Prize]) -> Option<usize> {
    let total = total_weight(prizes);
    if total == 0 {
        return None;
    }
    let roll = rand::thread_rng().gen_range(0..total);
    draw_with(prizes, roll)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prizes() -> Vec<Prize> {
        vec![
            Prize {
                id: 1,
                label: "100 coins".into(),
                weight: 70,
                remaining: 100,
            },
            Prize {
                id: 2,
                label: "500 coins".into(),
                weight: 25,
                remaining: 100,
            },
            Prize {
                id: 3,
                label: "limited edition skin".into(),
                weight: 5,
                remaining: 100,
            },
        ]
    }

    #[test]
    fn roll_lands_in_the_right_weight_bucket() {
        let p = prizes();
        // Intervals: [0,70) → 0, [70,95) → 1, [95,100) → 2
        for (roll, want) in [(0, 0), (69, 0), (70, 1), (94, 1), (95, 2), (99, 2)] {
            assert_eq!(draw_with(&p, roll), Some(want), "roll={roll}");
        }
    }

    /// Zero-stock prizes are excluded from the weight space — otherwise we'd draw unavailable stock,
    /// retry, and silently skew configured odds.
    #[test]
    fn sold_out_prizes_are_excluded_from_the_weight_space() {
        let mut p = prizes();
        p[0].remaining = 0;

        assert_eq!(total_weight(&p), 30, "total weight should be 25 + 5 only");
        for (roll, want) in [(0, 1), (24, 1), (25, 2), (29, 2)] {
            assert_eq!(draw_with(&p, roll), Some(want), "roll={roll}");
        }
    }

    #[test]
    fn zero_weight_prizes_are_never_drawn() {
        let mut p = prizes();
        p[2].weight = 0;

        for roll in 0..total_weight(&p) {
            assert_ne!(draw_with(&p, roll), Some(2), "weight-0 prize was drawn");
        }
    }

    #[test]
    fn everything_sold_out_yields_nothing() {
        let mut p = prizes();
        for x in &mut p {
            x.remaining = 0;
        }
        assert_eq!(draw_with(&p, 0), None);
        assert_eq!(draw(&p), None);
        assert_eq!(total_weight(&p), 0);
    }

    #[test]
    fn empty_pool_yields_nothing() {
        assert_eq!(draw_with(&[], 0), None);
        assert_eq!(draw(&[]), None);
    }

    /// Out-of-range roll must not panic or bias to an endpoint.
    #[test]
    fn out_of_range_rolls_wrap_around() {
        let p = prizes();
        assert_eq!(draw_with(&p, 100), draw_with(&p, 0));
        assert_eq!(draw_with(&p, -1), draw_with(&p, 99));
    }

    /// Live random path only guarantees "in range"; distribution covered by deterministic cases above.
    #[test]
    fn csprng_draw_stays_in_range() {
        let p = prizes();
        for _ in 0..200 {
            let i = draw(&p).expect("must return when stock exists");
            assert!(i < p.len());
            assert!(p[i].available());
        }
    }
}
