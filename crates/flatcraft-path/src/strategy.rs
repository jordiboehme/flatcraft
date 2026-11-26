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

/// The Fast strategy's host pivot threshold. Pinned by Task 10's
/// `pivot_threshold_choice`: the largest of 0, 0.5, 1, 2, 5 and 10 deg whose
/// rounded output stays within `DEVICE_LIMIT_MM` and whose float output
/// stays within `ALGORITHM_LIMIT_MM` (1 deg measures 0.0052 mm). Quality
/// and Outlines last keep 0, an arc at every corner: 0.5 deg saved 4 of
/// 4720 segments, too little to leave the simpler default.
pub const FAST_PIVOT_THRESHOLD_DEG: f64 = 0.5;

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
    /// Turns at or below this get no pivot arc in host compensation: 0 (an
    /// arc at every corner) or [`FAST_PIVOT_THRESHOLD_DEG`] for Fast.
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
                host_pivot_threshold_deg: FAST_PIVOT_THRESHOLD_DEG,
                ..quality
            },
            CutStrategy::OutlinesLast => Self {
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
        let k = StrategySettings::for_strategy(CutStrategy::OutlinesLast);
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
        }
        assert_eq!(q.host_pivot_threshold_deg, 0.0);
        assert_eq!(k.host_pivot_threshold_deg, 0.0);
        assert_eq!(f.host_pivot_threshold_deg, FAST_PIVOT_THRESHOLD_DEG);
    }
}
