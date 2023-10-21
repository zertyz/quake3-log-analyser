//! Resting place for [AnalysedEvent] & friends

use model::{
    types::Result,
    quake3_events::Quake3Events,
    report::GameMatchSummary,
};

/// Represents an event that might either be:
///   1) An unprocessed raw Quake3 game event,
///   2) An already processed event, upgraded to a Logic Event.\
/// Through this composite events, logic processing pipelines may be applied independently,
/// with each one working on a set of Game Events.
#[derive(Debug)]
pub enum CompositeEvent {
    GameEvent(Quake3Events),
    LogicEvent(LogicEvent)    // add line numbers
}

impl CompositeEvent {

    pub fn is_err(&self) -> bool {
        match self {
            CompositeEvent::GameEvent(quake3_event) => quake3_event.is_err(),
            CompositeEvent::LogicEvent(logic_event) => logic_event.is_err(),
        }
    }

    pub fn event_id(&self) -> u32 {
        match self {
            CompositeEvent::GameEvent(quake3_event) => quake3_event.event_id(),
            CompositeEvent::LogicEvent(logic_event) => logic_event.quake3_event_id(),
        }
    }
}

#[derive(Debug)]
pub struct AnalysedEvent {
    pub event_id:    usize,
    pub game_event:  Result<Quake3Events>, // should be Quake3Event
    pub logic_event: Option<LogicEvent>,
}

/// The events the main logic algorithms generates for the composable business logics to process
#[derive(Debug)]
pub enum LogicEvent {
    /// A game has started
    NewGame { quake3_event_id: u32 },
    /// A new player joined the game
    AddPlayer { quake3_event_id: u32, client_id: u32, name: String },
    /// An existing player changed its nick name
    RenamePlayer { quake3_event_id: u32, client_id: u32, old_name: String, new_name: String },
    /// An existing player quit the game
    DeletePlayer { quake3_event_id: u32, client_id: u32, name: String },
    /// Reports the cause of the last death
    MeanOfDeath { quake3_event_id: u32, mean_of_death: String },
    /// A player killed someone
    IncFrags { quake3_event_id: u32, client_id: u32, name: String },
    /// The player committed suicide (was killed by '<world>')
    DecFrags { quake3_event_id: u32, client_id: u32, name: String },
    /// The game reported its own account of a player's scored frags
    ReportedScore { quake3_event_id: u32, frags: i32, client_id: u32, name: String },
    /// A game has ended in a graceful manner: the match progressed until one of the limits were reached
    GameEndedGracefully { quake3_event_id: u32 },
    /// A game has ended without reaching any of the limits -- most likely due to an operator command
    GameEndedManually { quake3_event_id: u32 },

    /// Represents an error on the event processing
    EventModelViolation { quake3_event_id: u32, violation: EventModelViolations },
}

impl LogicEvent {

    pub fn is_err(&self) -> bool {
        matches!(self, LogicEvent::EventModelViolation {..})
    }

    pub fn quake3_event_id(&self) -> u32 {
        match self {
            LogicEvent::NewGame             { quake3_event_id, .. } |
            LogicEvent::AddPlayer           { quake3_event_id, .. } |
            LogicEvent::RenamePlayer        { quake3_event_id, .. } |
            LogicEvent::DeletePlayer        { quake3_event_id, .. } |
            LogicEvent::MeanOfDeath { quake3_event_id, .. } |
            LogicEvent::IncFrags            { quake3_event_id, .. } |
            LogicEvent::DecFrags            { quake3_event_id, .. } |
            LogicEvent::ReportedScore       { quake3_event_id, .. } |
            LogicEvent::GameEndedGracefully { quake3_event_id, .. } |
            LogicEvent::GameEndedManually   { quake3_event_id, .. } |
            LogicEvent::EventModelViolation { quake3_event_id, .. } => *quake3_event_id,
        }
    }
}

#[derive(Debug)]
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