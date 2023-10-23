//! Resting place for [Quake3Events]


use std::borrow::Cow;

/// Maps the Quake3 server events & info we care about, in close relation to [quake3-server-events::model::Quake3FullEvents].\
/// For detailed docs on each variant & field, please consult the referred object, which has the full picture.\
/// Every variant has an `event_id` -- it starts from 1 and references to the original events from the library.
///
/// IMPLEMENTATION NOTE: Notice this enum is similar to the one in the `quake3-server-events` crate.
/// Nonetheless, both should exist (regardless of the repetitiveness) for the following reasons:
///   1) `quake3-server-events` represents an external library, crafted for a different purpose than our business entities
///      -- having this model here decouples the application from the external library;
///   2) By unbinding the models, we end up having a simpler logic (as we don't need to know everything about all events)
#[derive(Debug)]
pub enum Quake3Events<'a> {
    InitGame              { event_id: u32 },
    ClientConnect         { event_id: u32, client_id: u32 },
    ClientUserinfoChanged { event_id: u32, client_id: u32, name: Cow<'a, str>},
    ClientDisconnect      { event_id: u32, client_id: u32 },
    Kill                  { event_id: u32, killer_id: u32, victim_id: u32, reason_id: u32, killer_name: Cow<'a, str>, victim_name: Cow<'a, str>, reason_name: Cow<'a, str> },
    Exit                  { event_id: u32 },
    Score                 { event_id: u32, frags: i32, client_id: u32, name: Cow<'a, str> },
    ShutdownGame          { event_id: u32 },
    Error                 { event_id: u32, err: Box<dyn std::error::Error> }
}

impl Quake3Events<'_> {

    /// Returns true if the event was not derived from an error
    pub fn is_ok(&self) -> bool {
        !matches!(self, Quake3Events::Error { .. })
    }

    /// Returns true if the event was derived from an error while processing the event source
    pub fn is_err(&self) -> bool {
        matches!(self, Quake3Events::Error { .. })
    }

    pub fn unwrap_err(&self) -> &Box<dyn std::error::Error> {
        if let Quake3Events::Error { event_id: _, err } = self {
            err
        } else {
            panic!("`Quake3Event` {self:?} is not an Error");
        }
    }

    /// Returns the `event_id` for the source Quake3 event
    pub fn event_id(&self) -> u32 {
        match self {
            Quake3Events::InitGame              { event_id, .. } |
            Quake3Events::ClientConnect         { event_id, .. } |
            Quake3Events::ClientUserinfoChanged { event_id, .. } |
            Quake3Events::ClientDisconnect      { event_id, .. } |
            Quake3Events::Kill                  { event_id, .. } |
            Quake3Events::Exit                  { event_id, .. } |
            Quake3Events::Score                 { event_id, .. } |
            Quake3Events::ShutdownGame          { event_id, .. } |
            Quake3Events::Error                 { event_id, .. } => *event_id
        }
    }

}