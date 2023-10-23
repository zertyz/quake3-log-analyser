//! Resting place for [LogicEvents], [CompositeEvent] & friends

use std::borrow::Cow;
use model::quake3_events::Quake3Events;


/// Represents an event that might either be:
///   1) An unprocessed raw Quake3 game event,
///   2) An already processed event, upgraded to a Logic Event.\
/// Through this composite events, logic processing pipelines may be applied independently,
/// with each one working on a set of Game Events.
#[derive(Debug)]
pub enum CompositeEvent<'a> {
    GameEvent(Quake3Events<'a>),
    LogicEvent(LogicEvents<'a>)
}

impl CompositeEvent<'_> {

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

/// The events the main logic algorithms generates for the composable business logics to process
#[derive(Debug)]
pub enum LogicEvents<'a> {
    /// A game has started
    NewGame { quake3_event_id: u32 },
    /// A new player joined the game
    AddPlayer { quake3_event_id: u32, client_id: u32, name: Cow<'a, str> },
    /// An existing player changed its nick name
    RenamePlayer { quake3_event_id: u32, client_id: u32, old_name: Cow<'a, str>, new_name: Cow<'a, str> },
    /// An existing player quit the game
    DeletePlayer { quake3_event_id: u32, client_id: u32, name: Cow<'a, str> },
    /// Reports the cause of the last death
    MeanOfDeath { quake3_event_id: u32, mean_of_death: Cow<'a, str> },
    /// A player killed someone
    IncFrags { quake3_event_id: u32, client_id: u32, name: Cow<'a, str> },
    /// The player committed suicide (was killed by '<world>')
    DecFrags { quake3_event_id: u32, client_id: u32, name: Cow<'a, str> },
    /// The game reported its own account of a player's scored frags
    ReportedScore { quake3_event_id: u32, frags: i32, client_id: u32, name: Cow<'a, str> },
    /// A game has ended in a graceful manner: the match progressed until one of the limits were reached
    GameEndedGracefully { quake3_event_id: u32 },
    /// A game has ended without reaching any of the limits -- most likely due to an operator command
    GameEndedManually { quake3_event_id: u32 },

    /// Represents an error on the event processing
    EventModelViolation { quake3_event_id: u32, violation: EventModelViolations<'a> },
}

impl LogicEvents<'_> {

    pub fn is_err(&self) -> bool {
        matches!(self, LogicEvents::EventModelViolation {..})
    }

    /// Returns the `event_id` of the variant of [Quake3Events] associated to this LogicEvents's variant
    /// -- which is, possibly, mapped to a line number from a log file.
    pub fn quake3_event_id(&self) -> u32 {
        match self {
            LogicEvents::NewGame             { quake3_event_id, .. } |
            LogicEvents::AddPlayer           { quake3_event_id, .. } |
            LogicEvents::RenamePlayer        { quake3_event_id, .. } |
            LogicEvents::DeletePlayer        { quake3_event_id, .. } |
            LogicEvents::MeanOfDeath { quake3_event_id, .. } |
            LogicEvents::IncFrags            { quake3_event_id, .. } |
            LogicEvents::DecFrags            { quake3_event_id, .. } |
            LogicEvents::ReportedScore       { quake3_event_id, .. } |
            LogicEvents::GameEndedGracefully { quake3_event_id, .. } |
            LogicEvents::GameEndedManually   { quake3_event_id, .. } |
            LogicEvents::EventModelViolation { quake3_event_id, .. } => *quake3_event_id,
        }
    }
}

/// Errors that may come after analysing [Quake3Events]
#[derive(Debug)]
pub enum EventModelViolations<'a> {
    /// Occurs when two [Quake3Events::InitGame] events were received before a [Quake3Events::ShutdownGame]
    DoubleInit,
    /// Occurs when two [[Quake3Events::ClientConnect]] events were received (for the same client_id) before a [Quake3Events::ClientDisconnect]
    DoubleConnect,
    /// Occurs when a game event happens outside of a game match (no [Quake3Events::InitGame] was issued)
    GameNotStarted,
    /// Occurs when a [Quake3Events::ClientUserinfoChanged] or [Quake3Events::ClientDisconnect] event happens before a [Quake3Events::ClientConnect], for the given client_id
    ClientNotConnected {
        id: u32,
        name: Cow<'a, str>,
    },
    /// Occurs when some game events report a name for a player, but others report other -- before a [Quake3Events::ClientUserinfoChanged] in between them
    DiscrepantPlayerName {
        id: u32,
        local_name: Cow<'a, str>,
        game_name: Cow<'a, str>,
    }
}