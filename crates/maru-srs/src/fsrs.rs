//! FSRS-6 memory model (Free Spaced Repetition Scheduler).
//!
//! Formulas and default parameters follow the reference implementation
//! (open-spaced-repetition/fsrs-rs, BSD-3-Clause); the test at the bottom
//! replays one of that crate's own test vectors.

pub const S_MIN: f32 = 0.001;
pub const S_MAX: f32 = 36500.0;
pub const D_MIN: f32 = 1.0;
pub const D_MAX: f32 = 10.0;

pub type Params = [f32; 21];

pub const DEFAULT_PARAMETERS: Params = [
    0.212, 1.2931, 2.3065, 8.2956, 6.4133, 0.8334, 3.0194, 0.001, 1.8722, 0.1666, 0.796, 1.4835, 0.0614, 0.2629,
    1.6483, 0.6014, 1.8729, 0.5425, 0.0912, 0.0658, 0.1542,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rating {
    Again = 1,
    Hard = 2,
    Good = 3,
    Easy = 4,
}

impl Rating {
    pub fn from_u8(n: u8) -> Option<Self> {
        match n {
            1 => Some(Rating::Again),
            2 => Some(Rating::Hard),
            3 => Some(Rating::Good),
            4 => Some(Rating::Easy),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Memory {
    pub stability: f32,
    pub difficulty: f32,
}

fn factor(w: &Params) -> f32 {
    let decay = -w[20];
    (0.9f32.ln() / decay).exp() - 1.0
}

/// Probability of recall `t` days after the last review.
pub fn retrievability(w: &Params, t: f32, s: f32) -> f32 {
    let decay = -w[20];
    (t / s * factor(w) + 1.0).powf(decay)
}

/// Days until recall probability falls to `desired_retention`.
pub fn next_interval(w: &Params, stability: f32, desired_retention: f32) -> f32 {
    let decay = -w[20];
    stability / factor(w) * (desired_retention.powf(1.0 / decay) - 1.0)
}

fn init_stability(w: &Params, rating: usize) -> f32 {
    w[rating.saturating_sub(1).min(3)]
}

fn init_difficulty(w: &Params, rating: usize) -> f32 {
    w[4] - (w[5] * rating.saturating_sub(1) as f32).exp() + 1.0
}

fn next_difficulty(w: &Params, d: f32, rating: f32) -> f32 {
    let delta = -w[6] * (rating - 3.0);
    let d = d + (10.0 - d) * delta / 9.0; // linear damping
    w[7] * (init_difficulty(w, 4) - d) + d // mean reversion
}

/// Apply one review. `elapsed_days` is whole days since the previous review;
/// `0` means a same-day (short-term) review. `None` memory means the first review.
pub fn review(w: &Params, memory: Option<Memory>, rating: Rating, elapsed_days: u32) -> Memory {
    let r = rating as usize;
    let Some(m) = memory else {
        return Memory {
            stability: init_stability(w, r).clamp(S_MIN, S_MAX),
            difficulty: init_difficulty(w, r).clamp(D_MIN, D_MAX),
        };
    };
    let s = m.stability.clamp(S_MIN, S_MAX);
    let d = m.difficulty.clamp(D_MIN, D_MAX);
    let rating_f = r as f32;
    let t = elapsed_days as f32;
    let ret = retrievability(w, t, s);

    let after_success = {
        let hard = if rating == Rating::Hard { w[15] } else { 1.0 };
        let easy = if rating == Rating::Easy { w[16] } else { 1.0 };
        s * (w[8].exp() * (11.0 - d) * s.powf(-w[9]) * (((1.0 - ret) * w[10]).exp() - 1.0) * hard * easy + 1.0)
    };
    let after_failure = {
        let new_s = w[11] * d.powf(-w[12]) * ((s + 1.0).powf(w[13]) - 1.0) * ((1.0 - ret) * w[14]).exp();
        new_s.min(s / (w[17] * w[18]).exp())
    };
    let short_term = {
        let inc = (w[17] * (rating_f - 3.0 + w[18])).exp() * s.powf(-w[19]);
        s * if rating_f >= 2.0 { inc.max(1.0) } else { inc }
    };

    let new_s = if elapsed_days == 0 {
        short_term
    } else if rating == Rating::Again {
        after_failure
    } else {
        after_success
    };
    Memory {
        stability: new_s.clamp(S_MIN, S_MAX),
        difficulty: next_difficulty(w, d, rating_f).clamp(D_MIN, D_MAX),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn replay(ratings: &[u8], intervals: &[u32]) -> Memory {
        let mut m = None;
        for (&r, &t) in ratings.iter().zip(intervals) {
            m = Some(review(&DEFAULT_PARAMETERS, m, Rating::from_u8(r).unwrap(), t));
        }
        m.unwrap()
    }

    /// Same sequence and expected values as the reference crate's `assert_memory_state` test.
    #[test]
    fn matches_reference_implementation() {
        let m = replay(&[1, 3, 3, 3, 3, 3], &[0, 0, 1, 3, 8, 21]);
        assert!((m.stability - 53.62691).abs() < 1e-2, "stability {}", m.stability);
        assert!((m.difficulty - 6.3574867).abs() < 1e-3, "difficulty {}", m.difficulty);
    }

    #[test]
    fn interval_for_90_percent_retention_equals_stability() {
        // By definition of stability: R(S) = 0.9.
        let w = DEFAULT_PARAMETERS;
        assert!((next_interval(&w, 10.0, 0.9) - 10.0).abs() < 1e-3);
        assert!((retrievability(&w, 10.0, 10.0) - 0.9).abs() < 1e-4);
    }

    #[test]
    fn success_grows_stability_and_failure_shrinks_it() {
        let w = DEFAULT_PARAMETERS;
        let m = Memory { stability: 20.0, difficulty: 5.0 };
        let good = review(&w, Some(m), Rating::Good, 20);
        let easy = review(&w, Some(m), Rating::Easy, 20);
        let hard = review(&w, Some(m), Rating::Hard, 20);
        let again = review(&w, Some(m), Rating::Again, 20);
        assert!(easy.stability > good.stability && good.stability > hard.stability);
        assert!(hard.stability > m.stability);
        assert!(again.stability < m.stability);
        assert!(again.difficulty > m.difficulty && easy.difficulty < m.difficulty);
    }

    #[test]
    fn first_review_uses_initial_values() {
        let m = review(&DEFAULT_PARAMETERS, None, Rating::Good, 0);
        assert!((m.stability - 2.3065).abs() < 1e-4);
        let again = review(&DEFAULT_PARAMETERS, None, Rating::Again, 0);
        assert!(again.difficulty > m.difficulty);
    }
}
