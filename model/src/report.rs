//! Contains summary data used to build reports

use std::collections::{BTreeSet, BTreeMap};

/// Grouped information
pub struct GroupedInformation {
    /// The [MatchSummary] for each of the available `game match id`s
    pub games: BTreeMap<u32, MatchSummary>,
}

pub struct MatchSummary {
    /// Sum of the frags of all players in [Self::kills]
    pub total_kills: u32,
    /// The name of the available players at the moment the match ended
    pub players: BTreeSet<String>,
    /// The frag score for each of the [Self::players].
    pub kills: BTreeMap<String, u32>,
}