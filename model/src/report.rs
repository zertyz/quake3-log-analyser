//! Contains summary data used to build reports

use std::collections::{BTreeSet, BTreeMap};
use std::pin::Pin;
use futures::Stream;

/// Grouped information for all matches / games available
pub type GamesSummary = Pin<Box<dyn Stream<Item=Result<GameMatchSummary, Box<dyn std::error::Error>>>>>;

/// Grouped information for a single match / game
#[derive(Debug,PartialEq)]
pub struct GameMatchSummary {
    /// Sum of the frags of all players in [Self::kills]
    pub total_kills: u32,
    /// The name of the available players at the moment the match ended
    pub players: BTreeSet<String>,
    /// The frag score for each of the [Self::players].
    pub kills: BTreeMap<String, i32>,

    /// extended / optional fields
    //////////////////////////////

    /// The number of casualties caused by each reasons
    pub means_of_death: Option<BTreeMap<String, i32>>,
    /// The score the server reports through `score` events
    pub game_reported_scores: Option<BTreeMap<String, i32>>,
    /// Vector of users who disconnected before the game ended,
    /// in the form (id, nick, frags)
    pub disconnected_players: Option<Vec<(u32, String, i32)>>
}