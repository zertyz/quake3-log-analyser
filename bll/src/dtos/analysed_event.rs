//! Resting place for [AnalysedEvent] & friends

use model::{
    types::Result,
    report::GameMatchSummary,
};
use quake3_server_log::model::Quake3Events;


pub enum CompositeEvent {
    GameEvent(Result<Quake3Events>),
    LogicEvent(BllEvent)    // add line numbers
}

pub struct AnalysedEvent {
    pub event_id:    usize,
    pub game_event:  Result<Quake3Events>, // should be Quake3Event
    pub logic_event: Option<BllEvent>,
}

/// The events the main logic algorithms generates for the composable business logics to process
pub enum BllEvent {
    /// A game has started
    NewGame,
    /// A new player joined the game
    AddPlayer { id: u32, name: String },
    /// An existing player changed its nick name
    RenamePlayer { id: u32, old_name: String, new_name: String },
    /// An existing player quit the game
    DeletePlayer { id: u32, name: String },
    /// A player killed someone
    IncFrags { id: u32, name: String },
    /// The player committed suicide (was killed by '<world>')
    DecFrags { id: u32, name: String },
    /// A game has ended in a graceful manner: the match progressed until one of the limits were reached
    GameEndedGracefully,
    /// A game has ended without reaching any of the limits -- most likely due to an operator command
    GameEndedManually,

    EventModelViolation { violation: EventModelViolations },
}

pub enum EventModelViolations {
    /// Occurs when two `InitGame` events were received before a `ShutdownGame`
    DoubleInit,
    /// Occurs when two `ClientConnect` events were received (for the same client_id) before a `ClientDisconnect`
    DoubleConnect,
    /// Occurs when a game event happens outside of a game match (no `InitGame` was issued)
    GameNotStarted,
    /// Occurs when a `ClientUserinfoChanged` or `ClientDisconnect` event happens before a `ClientConnect`, for the given client_id
    ClientNotConnected {
        id: u32,
        name: String,
    },
    DiscrepantPlayerName {
        id: u32,
        local_name: String,
        game_name: String,
    }
}