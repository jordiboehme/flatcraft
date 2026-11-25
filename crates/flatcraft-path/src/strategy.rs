//! Cut strategies: what the cut order optimises for (spec 8.3, R8).
//!
//! Every strategy uses the same entry candidates, keeps the kept-side
//! penalty and cuts every child before its parent. They differ in how much
//! a blade turn costs and in whether outlines wait for everything else.

use flatcraft_core::document::CutStrategy;

use crate::order::BUDGET;

/// What one radian of blade turn at an entry costs, in millimetres of travel.
pub const TURN_WEIGHT: f64 = 2.0;

/// How many improvement rounds the order runs at most (Task 6).
pub const ROUNDS: u32 = 3;

/// The order settings of a [`CutStrategy`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StrategySettings {
    /// Every path inside another comes before any outline (root path).
    pub outlines_last: bool,
    /// See [`TURN_WEIGHT`].
    pub turn_weight_mm_per_rad: f64,
    /// Move and edge evaluations the improvement may spend (Task 6).
    pub budget: u64,
    /// See [`ROUNDS`].
    pub rounds: u32,
    /// Turns below this get no pivot arc. Task 10 tunes it; 0 until then
    /// (an arc at every corner).
    pub host_pivot_threshold_deg: f64,
    /// Or-opt moves in the improvement (Task 6). Internal: not in
    /// [`CutStrategy`] or the UI (Ruling F6).
    pub or_opt: bool,
    /// The exact entry-point DP in the improvement (Task 6). Internal.
    pub entry_dp: bool,
    /// The crossing penalty in the construction. Internal.
    pub crossing_penalty: bool,
}

impl StrategySettings {
    pub fn for_strategy(strategy: CutStrategy) -> Self {
        let quality = Self {
            outlines_last: false,
            turn_weight_mm_per_rad: TURN_WEIGHT,
            budget: BUDGET,
            rounds: ROUNDS,
            host_pivot_threshold_deg: 0.0,
            or_opt: true,
            entry_dp: true,
            crossing_penalty: true,
        };
        match strategy {
            CutStrategy::Quality => quality,
            CutStrategy::Fast => Self {
                turn_weight_mm_per_rad: 0.0,
                ..quality
            },
            CutStrategy::KeepSheet => Self {
                outlines_last: true,
                ..quality
            },
        }
    }
}

impl Default for StrategySettings {
    fn default() -> Self {
        Self::for_strategy(CutStrategy::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Structural: restates the strategy table.
    #[test]
    fn strategies_differ_only_where_the_table_says() {
        let q = StrategySettings::for_strategy(CutStrategy::Quality);
        let f = StrategySettings::for_strategy(CutStrategy::Fast);
        let k = StrategySettings::for_strategy(CutStrategy::KeepSheet);
        assert_eq!(q, StrategySettings::default());
        assert_eq!(
            (q.turn_weight_mm_per_rad, q.outlines_last),
            (TURN_WEIGHT, false)
        );
        assert_eq!((f.turn_weight_mm_per_rad, f.outlines_last), (0.0, false));
        assert_eq!(
            (k.turn_weight_mm_per_rad, k.outlines_last),
            (TURN_WEIGHT, true)
        );
        for s in [q, f, k] {
            assert_eq!((s.budget, s.rounds), (BUDGET, ROUNDS));
            assert!(s.or_opt && s.entry_dp && s.crossing_penalty);
            assert_eq!(s.host_pivot_threshold_deg, 0.0);
        }
    }
}
