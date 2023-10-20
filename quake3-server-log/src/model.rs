//! Resting place for [Quake3Events]

/// Mappings for Quake 3 server events
#[derive(Debug, PartialEq)]
pub enum Quake3Events {
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
        name: String,

    },
    ClientBegin  {
        id: u32,
    },
    ClientDisconnect {
        id: u32,
    },
    Item,
    Say,
    Kill {
        killer_id: u32,
        victim_id: u32,
        reason_id: u32,
        killer_name: String,
        victim_name: String,
        reason_name: String,
    },
    Exit,
    CaptureTheFlagResults {
        red: u32,
        blue: u32,
    },
    Score {
        frags: i32,
        id: u32,
        name: String,
    },
    ShutdownGame,
    Comment,
}