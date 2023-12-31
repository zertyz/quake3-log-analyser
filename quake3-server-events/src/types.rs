//! Resting place for [Quake3FullEvents]

use std::borrow::Cow;

/// Mappings for Quake 3 server events.\
/// IMPLEMENTATION NOTE: The name says "full", despite this not being true, but the term is just to emphasize
/// that this library represents an external piece of code without any relation to our Business Logic requisites.
#[derive(Debug, PartialEq)]
pub enum Quake3FullEvents<'a> {
    /// A new game match has started
    InitGame {
        /// Applicable to the "Deathmatch" mode, specifies the maximum score (frag) a player may have -- after which, the match is declared over
        frag_limit: Option<u32>,
        /// Applicable to the "Capture the flag" mode, specifies the limit score -- after which the match is declared over
        capture_limit:  Option<u32>,
        /// Applicable to both modes, specifies the maximum duration for the match, in minutes
        time_limit_min: Option<u32>,
    },
    /// A player has just connected
    ClientConnect {
        id: u32,
    },
    /// An update on the player's info is available
    ClientUserinfoChanged {
        id: u32,
        name: Cow<'a, str>,

    },
    /// Client started playing
    ClientBegin  {
        id: u32,
    },
    /// Client quit the game
    ClientDisconnect {
        id: u32,
    },
    /// Client grab an item
    Item,
    /// Client sent a chat
    Say,
    /// Client killed someone or died due to injuries / suicide
    Kill {
        killer_id: u32,
        victim_id: u32,
        reason_id: u32,
        killer_name: Cow<'a, str>,
        victim_name: Cow<'a, str>,
        reason_name: Cow<'a, str>,
    },
    /// Graceful game finish
    Exit,
    /// Scores for capture the flag games
    CaptureTheFlagResults {
        red: u32,
        blue: u32,
    },
    /// Scores for Deathmatch games
    Score {
        frags: i32,
        id: u32,
        name: Cow<'a, str>,
    },
    /// Game is over
    ShutdownGame,
    /// Log message that shares no event
    Comment,
}