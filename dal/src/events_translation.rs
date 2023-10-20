//! Contains utilities for translating the outputs of the `quake3-server-events`
//! library into our simplified models for the events and info we care about

use std::future;
use model::{
    types::Result,
    quake3_events::Quake3Events,
};
use quake3_server_log::model::Quake3FullEvents;
use futures::{Stream, StreamExt};


/// Receives a `Stream` of the Quake3 events produced by the `quake3-server-events` library and
/// simplifies & translates them into another `Stream` of our [model::quake3_events::Quake3Events]
pub fn translate_quake3_events(lib_events: impl Stream<Item=Result<Quake3FullEvents>>) -> impl Stream<Item=Quake3Events> {
    let mut event_id = 0;
    lib_events
        .map(move |event_result| {
            event_id += 1;
            let Ok(event) = event_result
                else {
                    return Some(Quake3Events::Error { event_id, err: event_result.unwrap_err() })
                };
            match event {
                Quake3FullEvents::InitGame { .. } => Some(Quake3Events::InitGame { event_id }),
                Quake3FullEvents::ClientConnect { id } => Some(Quake3Events::ClientConnect { event_id, client_id: id }),
                Quake3FullEvents::ClientUserinfoChanged { id, name } => Some(Quake3Events::ClientUserinfoChanged { event_id, client_id: id, name }),
                Quake3FullEvents::ClientBegin { .. } => None,
                Quake3FullEvents::ClientDisconnect { id } => Some(Quake3Events::ClientDisconnect { event_id, client_id: id }),
                Quake3FullEvents::Item => None,
                Quake3FullEvents::Say => None,
                Quake3FullEvents::Kill { killer_id, victim_id, reason_id, killer_name, victim_name, reason_name } => Some(Quake3Events::Kill { event_id, killer_id, victim_id, reason_id, killer_name, victim_name, reason_name }),
                Quake3FullEvents::Exit => Some(Quake3Events::Exit { event_id }),
                Quake3FullEvents::CaptureTheFlagResults { .. } => None,
                Quake3FullEvents::Score { frags, id, name } => Some(Quake3Events::Score { event_id, frags, client_id: id, name }),
                Quake3FullEvents::ShutdownGame => Some(Quake3Events::ShutdownGame { event_id }),
                Quake3FullEvents::Comment => None,
            }
        })
        .filter_map(|our_event_option| future::ready(our_event_option))
}