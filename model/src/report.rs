//! Contains summary data used to build reports

use std::collections::{BTreeSet, BTreeMap};

/// Grouped information for all matches / games
#[derive(Debug,PartialEq)]
pub struct GamesSummary {
    /// The [GameMatchSummary] for each of the available `game match id`s
    pub games: BTreeMap<u32, GameMatchSummary>,
}

/// Grouped information for a single match / game
#[derive(Debug,PartialEq)]
pub struct GameMatchSummary {
    /// Sum of the frags of all players in [Self::kills]
    pub total_kills: u32,
    /// The name of the available players at the moment the match ended
    pub players: BTreeSet<String>,
    /// The frag score for each of the [Self::players].
    pub kills: BTreeMap<String, i32>,

    pub scores: Option<BTreeMap<String, i32>>,
}