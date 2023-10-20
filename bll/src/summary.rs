//! Builds a summaries of Quake3 game matches.\
//! Here you'll find an event-based, decoupled and zero-cost-abstraction strategy for applying business logic rules & requisites:
//!   1) [Quake3Events] events come in in a `Stream` and [GameMatchSummary] events goes out, also in a `Stream`
//!   2) Logic processors can be enabled / disabled by adding `Stream` operations -- only pay for what you use
//!   3) The `Stream` operations are nicely packed into their own functions, enabling an easy selection through [Config::processor_pipeline]
//! The strategy for applying business logic

use model::{
    types::Result,
    quake3_events::Quake3Events,
    report::GameMatchSummary,
};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::future;
use std::pin::Pin;
use std::sync::Arc;
use futures::{Stream, stream, StreamExt};
use dal_api::Quake3ServerEvents;
use log::warn;
use model::report::GamesSummary;
use crate::{Config, EventAnalyserOperations};
use crate::dtos::{AnalysedEvent, LogicEvent, CompositeEvent, EventModelViolations};


/// The basis for the logic operations: Upgrades the Quake3 events into a [CompositeEvent], from which we may
/// aggregate many logic processing pipelines.\
/// The workings of the processing pipelines are as follows:
///   1. `Stream` of [Quake3Events], then
///   2. [compose()], then
///   3. many pipeline processing functions, such as [kills()], [player_ids_and_nicknames_resolutions()] and [game_reported_scores()] -- then
///   4.  [summarize()], then
///   5. `Stream` of [GameMatchSummary]
pub fn compose(config: Arc<Config>, log_dao: impl Quake3ServerEvents) -> Result<impl Stream<Item=CompositeEvent>> {

    let stream = log_dao.events_stream()
        .map_err(|err| format!("summarize_games(): failed at fetching the events `Stream`: {err}"))?;

    let stream = stream
        .inspect(move |quake3_event| if config.log_issues {
            if let Quake3Events::Error {event_id, err} = quake3_event {
                warn!("Failed to process event #{event_id}: {err}");
            }
        });

    let mut in_game = false;
    let mut graceful_game_end = false;

    let stream = stream
        .map(move |quake3_event| {
            match &quake3_event {

                Quake3Events::InitGame { event_id } => {
                    if in_game {
                        Some(CompositeEvent::LogicEvent(LogicEvent::EventModelViolation { quake3_event_id: *event_id, violation: EventModelViolations::DoubleInit}))
                    } else {
                        in_game = true;
                        graceful_game_end = false;
                        Some(CompositeEvent::LogicEvent(LogicEvent::NewGame { quake3_event_id: *event_id }))
                    }
                },

                Quake3Events::Exit { event_id } => {
                    if in_game {
                        graceful_game_end = true;
                        None
                    } else {
                        Some(CompositeEvent::LogicEvent(LogicEvent::EventModelViolation { quake3_event_id: *event_id, violation: EventModelViolations::GameNotStarted }))
                    }
                }

                Quake3Events::ShutdownGame { event_id } => {
                    if in_game {
                        in_game = false;
                        if graceful_game_end {
                            Some(CompositeEvent::LogicEvent(LogicEvent::GameEndedGracefully { quake3_event_id: *event_id }))
                        } else {
                            Some(CompositeEvent::LogicEvent(LogicEvent::GameEndedManually { quake3_event_id: *event_id }))
                        }
                    } else {
                        Some(CompositeEvent::LogicEvent(LogicEvent::EventModelViolation { quake3_event_id: *event_id, violation: EventModelViolations::GameNotStarted }))
                    }
                },

                Quake3Events::Error { event_id, err } => {
                    // upgrades the error description
                    let err = Box::from(format!("Event #{event_id}: Feed error: {err}"));
                    Some(CompositeEvent::GameEvent(Quake3Events::Error { event_id: *event_id, err }))
                },

                _ => Some(CompositeEvent::GameEvent(quake3_event))
            }
        })
        .filter_map(|composite_event_option| future::ready(composite_event_option));
    Ok(stream)

}

/// Logic for "kill" events -- killers get a frag up; if killed by '<world>', the victim gets a frag down.\
/// NOTE: should be applied before [player_ids_and_nicknames_resolutions()]
pub fn kills(config: Arc<Config>, stream: impl Stream<Item=CompositeEvent>) -> impl Stream<Item=CompositeEvent> {

    stream
        .map(|composite_event| {

            // game events -- map some of the Quake3 events to `BllEvent::IncFrags`, `BllEvent::DecFrags`,
            let CompositeEvent::GameEvent(ref game_event) = composite_event
                else {
                    return Some(composite_event)
                };

            match game_event {

                Quake3Events::Kill { event_id, killer_id, victim_id, reason_id, killer_name, victim_name, reason_name } => {
                    if killer_name != "<world>" {
                        Some(CompositeEvent::LogicEvent(LogicEvent::IncFrags { quake3_event_id: *event_id, client_id: *killer_id, name: killer_name.to_owned() }))
                    } else {
                        Some(CompositeEvent::LogicEvent(LogicEvent::DecFrags { quake3_event_id: *event_id, client_id: *victim_id, name: victim_name.to_owned() }))
                    }
                },

                _ => Some(composite_event)
            }
        })
        .filter_map(|composite_event_option| future::ready(composite_event_option))

}

/// Logic for resolving client ids & client names & validating the ones resolved by the game.\
/// NOTE: should be applied after [kills()]
pub fn player_ids_and_nicknames_resolutions(config: Arc<Config>, stream: impl Stream<Item=CompositeEvent>) -> impl Stream<Item=CompositeEvent> {

    let mut player_ids_and_nicks = HashMap::<u32, Option<String>>::new();

    stream
        .map(move |composite_event| {

            // logic events: verify there are no nick names discrepancies
            if let CompositeEvent::LogicEvent(ref logic_event) = composite_event {

                // common code for the match arms bellow: reports if there are discrepancies in the player names for Inc and Dec frag events
                let react_to_nicknames_discrepancy = |quake3_event_id, id, name: String, composite_event| {
                    player_ids_and_nicks.get(&id)
                        .and_then(|stored_name| if stored_name == &Some(name.clone()) {
                                Some(composite_event)
                            } else {
                                Some(CompositeEvent::LogicEvent(
                                    LogicEvent::EventModelViolation {
                                        quake3_event_id,
                                        violation: EventModelViolations::DiscrepantPlayerName {
                                            id,
                                            local_name: stored_name.as_ref().unwrap_or(&"NONE".to_owned()).to_owned(),
                                            game_name: name,
                                        }
                                    }
                                ))
                            }
                        )
                };

                return match logic_event {
                    LogicEvent::NewGame { .. } => {
                        player_ids_and_nicks.clear();
                        Some(composite_event)
                    },
                    LogicEvent::IncFrags { quake3_event_id, client_id: id, name } if config.stop_on_event_model_violations => react_to_nicknames_discrepancy(*quake3_event_id, *id, name.to_owned(), composite_event),
                    LogicEvent::DecFrags { quake3_event_id, client_id: id, name } if config.stop_on_event_model_violations => react_to_nicknames_discrepancy(*quake3_event_id, *id, name.to_owned(), composite_event),
                    _ => Some(composite_event)
                }
            }

            // game events -- map some of the Quake3 events to `BllEvent::AddPlayer`, `BllEvent::RenamePlayer` & `BllEvent::DeletePlayer`
            let CompositeEvent::GameEvent(ref game_event) = composite_event
                else {
                    return Some(composite_event)
                };
            match game_event {

                Quake3Events::ClientConnect { event_id, client_id: id } => {
                    player_ids_and_nicks.insert(*id, None)
                        .map_or_else(|| None,
                                     //|old_nick| Some(Err(Box::from(format!("Event #{}: Two `ClientConnect {{id: {id}}}` events received before a `ClientDisconnect` -- '{old_nick}' was there already", event_id+1)))))
                                     |old_nick| Some(CompositeEvent::LogicEvent(LogicEvent::EventModelViolation { quake3_event_id: *event_id, violation: EventModelViolations::DoubleConnect})))
                },

                Quake3Events::ClientUserinfoChanged { event_id, client_id: id, name: new_name } => {
                    player_ids_and_nicks.get_mut(&id)
                        //.map_or_else(|| Some(Err(Box::from(format!("Event #{}: `ClientUserinfoChanged` event received before a `ClientConnect`", event_id+1)))),
                        .map_or_else(|| Some(CompositeEvent::LogicEvent(LogicEvent::EventModelViolation { quake3_event_id: *event_id, violation: EventModelViolations::ClientNotConnected {id: *id, name: new_name.to_owned()}})),
                                     |old_name| old_name.replace(new_name.clone())
                                             .and_then(|old_name| Some(CompositeEvent::LogicEvent(LogicEvent::RenamePlayer { quake3_event_id: *event_id, client_id: *id, old_name, new_name: new_name.clone() })) )
                                             .or_else(|| Some(CompositeEvent::LogicEvent(LogicEvent::AddPlayer { quake3_event_id: *event_id, client_id: 0, name: new_name.to_owned() })) ) )
                },

                Quake3Events::ClientDisconnect { event_id, client_id: id } => {
                    player_ids_and_nicks.remove(id)
                        .and_then(|name| Some(CompositeEvent::LogicEvent(LogicEvent::DeletePlayer { quake3_event_id: *event_id, client_id: *id, name: name.unwrap_or("NONE".to_owned())})))
                        .or_else(|| Some(CompositeEvent::LogicEvent(LogicEvent::EventModelViolation { quake3_event_id: *event_id, violation: EventModelViolations::ClientNotConnected {id: *id, name: "<unknown>".to_owned()}})))
                }

                _ => Some(composite_event)
            }
        })
        .filter_map(|composite_event_option| future::ready(composite_event_option))

}

/// Logic for resolving player scores reported by the game
pub fn game_reported_scores(config: Arc<Config>, stream: impl Stream<Item=CompositeEvent>) -> impl Stream<Item=CompositeEvent> {

    stream
        .map(|composite_event| {

            // game events -- map the Quake3 `Score` event into `BllEvent::ReportedScore`
            let CompositeEvent::GameEvent(ref game_event) = composite_event
                else {
                    return Some(composite_event)
                };

            match game_event {

                Quake3Events::Score { event_id, frags, client_id, name } =>
                    Some(CompositeEvent::LogicEvent(LogicEvent::ReportedScore { quake3_event_id: *event_id, frags: *frags, client_id: *client_id, name: name.to_owned() })),

                _ => Some(composite_event)
            }
        })
        .filter_map(|composite_event_option| future::ready(composite_event_option))
}

/// Ties together the Logic Events in the operated `stream` into a [GameMatchSummary] ready to be presented to the user,
/// ignoring any unprocessed Game Events that might be left (due to a modest chaining of operations).\
/// See [compose()] for a full description of the process.
pub fn summarize(config: Arc<Config>, stream: impl Stream<Item=CompositeEvent>) -> impl Stream<Item=Result<GameMatchSummary>> {

    let mut current_game_summary = None;


    stream
        .map(move |composite_event| {

            // process only logic events
            if let CompositeEvent::LogicEvent(logic_event) = composite_event {
                match logic_event {

                    LogicEvent::NewGame { quake3_event_id } => {
                        current_game_summary
                            .replace(GameMatchSummary {
                                total_kills: 0,
                                players: BTreeSet::new(),
                                kills: BTreeMap::new(),
                                game_reported_scores: None,
                                disconnected_players: None,
                            })
                            .map_or_else(|| None,
                                         |previous| Some(Err(Box::from(format!("Quake3 Event #{quake3_event_id}: Two `InitGame` events received before a `ShutdownGame`")))) )
                    },

                    LogicEvent::AddPlayer { quake3_event_id, client_id: id, name } =>
                        if let Some(ref mut current_game_summary) = current_game_summary {
                            (!current_game_summary.players.insert(name.clone()))
                                .then(|| Err(Box::from(format!("Event #{quake3_event_id}: Player id: {id}, name: {name:?} is already registered"))))
                        } else {
                            // err: no new game event
                            None
                        },

                    LogicEvent::RenamePlayer { quake3_event_id, client_id: id, old_name, new_name } =>
                        if let Some(ref mut current_game_summary) = current_game_summary {
                            current_game_summary.players.remove(&old_name);
                            current_game_summary.players.insert(new_name.clone());
                            current_game_summary.kills.remove(&old_name)
                                .and_then(|frags| current_game_summary.kills.insert(new_name.clone(), frags));
                            None
                        } else {
                            // err: no new game event
                            None
                        },

                    LogicEvent::DeletePlayer { quake3_event_id, client_id: id, name } =>
                        if let Some(ref mut current_game_summary) = current_game_summary {
                            current_game_summary.kills.remove(&name)
                                .map(|frags| current_game_summary.disconnected_players.get_or_insert_with(|| Vec::new())
                                    .push( (id, name.clone(), frags) ));
                            (!current_game_summary.players.remove(&name))
                                .then(|| Err(Box::from(format!("Event #{quake3_event_id}: Player id: {id}, name: {name:?} was not registered"))))
                        } else {
                            // err: no new game event
                            None
                        },

                    LogicEvent::IncFrags { quake3_event_id, client_id: id, name } =>
                        if let Some(ref mut current_game_summary) = current_game_summary {
                            current_game_summary.total_kills += 1;
                            current_game_summary.players.insert(name.clone());
                            current_game_summary.kills.entry(name)
                                .and_modify(|frags| *frags += 1)
                                .or_insert(1);
                            None
                        } else {
                            // err: no new game event
                            None
                        },

                    LogicEvent::DecFrags { quake3_event_id, client_id: id, name } =>
                        if let Some(ref mut current_game_summary) = current_game_summary {
                            current_game_summary.total_kills += 1;
                            current_game_summary.players.insert(name.clone());
                            current_game_summary.kills.entry(name)
                                .and_modify(|frags| *frags -= 1)
                                .or_insert(-1);
                            None
                        } else {
                            // err: no new game event
                            None
                        },

                    LogicEvent::ReportedScore { quake3_event_id, frags, client_id, name } =>
                        if let Some(ref mut current_game_summary) = current_game_summary {
                            current_game_summary.game_reported_scores.get_or_insert_with(|| BTreeMap::new())
                                .insert(name, frags);
                            None
                        } else {
                            // err: no new game event
                            None
                        },

                    LogicEvent::GameEndedManually { quake3_event_id } =>
                        Some(current_game_summary.take()
                            .ok_or_else(|| Box::from(format!("Event #{quake3_event_id}: Game ended, but it was never started"))) ),

                    LogicEvent::GameEndedGracefully { quake3_event_id } =>
                        Some(current_game_summary.take()
                            .ok_or_else(|| Box::from(format!("Event #{quake3_event_id}: Game ended gracefully, but it was never started"))) ),

                    LogicEvent::EventModelViolation { quake3_event_id, violation } =>
                        Some(Err(Box::from(format!("Event #{quake3_event_id}: violated the event model: {violation:?}")))),

                    _ => None
                }
            } else {
                // ignore any remaining Game Events
                None
            }

        })
        .filter_map(|composite_event_option| future::ready(composite_event_option))
}

pub fn summarize_games(config: Arc<Config>, log_dao: impl Quake3ServerEvents + 'static) -> Result<GamesSummary> {
    let stream = compose(config.clone(), log_dao)?;
    if config.processor_pipeline == HashSet::from([EventAnalyserOperations::Kills]) {
        return Ok(Box::pin(summarize(config.clone(), kills(config.clone(), stream))))
    } else if config.processor_pipeline == HashSet::from([EventAnalyserOperations::Kills, EventAnalyserOperations::PlayerIdsAndNickNamesResolutions, EventAnalyserOperations::GameReportedScores]) {
        return Ok(Box::pin(summarize(config.clone(),  game_reported_scores(config.clone(), player_ids_and_nicknames_resolutions(config.clone(), kills(config.clone(), stream))))))
    } else if config.processor_pipeline == HashSet::from([EventAnalyserOperations::Kills, EventAnalyserOperations::PlayerIdsAndNickNamesResolutions]) {
        return Ok(Box::pin(summarize(config.clone(),  player_ids_and_nicknames_resolutions(config.clone(), kills(config.clone(), stream)))))
    } else if config.processor_pipeline == HashSet::from([EventAnalyserOperations::Kills, EventAnalyserOperations::GameReportedScores]) {
        return Ok(Box::pin(summarize(config.clone(),  game_reported_scores(config.clone(), kills(config.clone(), stream)))))
    } else {
        Err(Box::from(format!("Summary Logic: Unknown combination of logic operations for the processor_pipeline {:?}", config.processor_pipeline)))
    }
}

/// Tests the [summary](super) logic module
#[cfg(test)]
mod tests {
    use dal::sync_file_reader::Quake3LogFileSyncReader;
    use super::*;


    // unit-isolated tests section
    //////////////////////////////
    // the following tests use a mock implementation for the DAL layer: `TestDAL`,
    // allowing us freedom to test some simple, yet diverse set of scenarios

    #[test]
    fn composition() {
        let events = vec![
            Quake3Events::InitGame     { event_id: 1 },
            Quake3Events::Kill         { event_id: 2, killer_id: 1, victim_id: 2, reason_id: 1, killer_name: "Player1".to_owned(), victim_name: "Player2".to_owned(), reason_name: "NONE".to_owned() },
            Quake3Events::Kill         { event_id: 3, killer_id: 2, victim_id: 1, reason_id: 2, killer_name: "Player2".to_owned(), victim_name: "Player1".to_owned(), reason_name: "NONE".to_owned() },
            Quake3Events::ShutdownGame { event_id: 4 },
            Quake3Events::InitGame     { event_id: 1 },
            Quake3Events::Kill         { event_id: 2, killer_id: 1, victim_id: 2, reason_id: 1, killer_name: "Player1".to_owned(), victim_name: "Player2".to_owned(), reason_name: "NONE".to_owned() },
            Quake3Events::Kill         { event_id: 3, killer_id: 2, victim_id: 1, reason_id: 2, killer_name: "Player2".to_owned(), victim_name: "Player1".to_owned(), reason_name: "NONE".to_owned() },
            Quake3Events::ShutdownGame { event_id: 4 },
        ];
        let log_dao = TestDAL::new(events);
        let composite_events = compose(test_config(), log_dao).expect("compose() shouldn't fail here");
        for e in futures::executor::block_on_stream(composite_events) {
            println!("{e:?}");
            assert!(!e.is_err(), "Model violation wrongly detected while processing event #{}", e.event_id());
        }
    }

    #[test]
    fn happy_path() {
        let events = vec![
            Quake3Events::InitGame     { event_id: 1 },
            Quake3Events::Kill         { event_id: 2, killer_id: 1, victim_id: 2, reason_id: 1, killer_name: "Player1".to_owned(), victim_name: "Player2".to_owned(), reason_name: "NONE".to_owned() },
            Quake3Events::Kill         { event_id: 3, killer_id: 2, victim_id: 1, reason_id: 2, killer_name: "Player2".to_owned(), victim_name: "Player1".to_owned(), reason_name: "NONE".to_owned() },
            Quake3Events::ShutdownGame { event_id: 4 },
        ];
        let expected_summaries = vec![
            GameMatchSummary {
                total_kills: 2,
                players: BTreeSet::from([
                    "Player1".to_owned(),
                    "Player2".to_owned(),
                ]),
                kills: BTreeMap::from([
                    ("Player1".to_owned(), 1),
                    ("Player2".to_owned(), 1),
                ]),
                game_reported_scores: None,
                disconnected_players: None,
            },
        ];
        assert_mock_summaries(events, expected_summaries)
    }

    /// Assures `<world>` kills discount 1 on the score of the victim players,
    /// possibly yielding to negative scores
    #[test]
    fn world_kills() {

        // scenario: only negative scores
        /////////////////////////////////

        let events = vec![
            Quake3Events::InitGame     { event_id: 1 } ,
            Quake3Events::Kill         { event_id: 2, killer_id: 1022, victim_id: 2, reason_id: 1, killer_name: "<world>".to_owned(), victim_name: "Player2".to_owned(), reason_name: "NONE".to_owned() },
            Quake3Events::Kill         { event_id: 3, killer_id: 2022, victim_id: 1, reason_id: 2, killer_name: "<world>".to_owned(), victim_name: "Player1".to_owned(), reason_name: "NONE".to_owned() },
            Quake3Events::Kill         { event_id: 4, killer_id: 2022, victim_id: 1, reason_id: 2, killer_name: "<world>".to_owned(), victim_name: "Player1".to_owned(), reason_name: "NONE".to_owned() },
            Quake3Events::ShutdownGame { event_id: 5 },
        ];
        let expected_summaries = vec![
            GameMatchSummary {
                total_kills: 3,
                players: BTreeSet::from([
                    "Player1".to_owned(),
                    "Player2".to_owned(),
                ]),
                kills: BTreeMap::from([
                    ("Player1".to_owned(), -2),
                    ("Player2".to_owned(), -1),
                ]),
                game_reported_scores: None,
                disconnected_players: None,
            },
        ];
        assert_mock_summaries(events, expected_summaries);

        // scenario: positives and negatives for a zero net result
        //////////////////////////////////////////////////////////

        let events = vec![
            Quake3Events::InitGame     { event_id: 1 },
            Quake3Events::Kill         { event_id: 2, killer_id: 1022, victim_id: 2, reason_id: 1, killer_name: "<world>".to_owned(), victim_name: "Player2".to_owned(), reason_name: "NONE".to_owned() },
            Quake3Events::Kill         { event_id: 3, killer_id: 2022, victim_id: 1, reason_id: 2, killer_name: "<world>".to_owned(), victim_name: "Player1".to_owned(), reason_name: "NONE".to_owned() },
            Quake3Events::Kill         { event_id: 4, killer_id: 2022, victim_id: 1, reason_id: 2, killer_name: "<world>".to_owned(), victim_name: "Player1".to_owned(), reason_name: "NONE".to_owned() },
            Quake3Events::Kill         { event_id: 5, killer_id: 1, victim_id: 2, reason_id: 1, killer_name: "Player1".to_owned(), victim_name: "Player2".to_owned(), reason_name: "NONE".to_owned() },
            Quake3Events::Kill         { event_id: 6, killer_id: 2, victim_id: 1, reason_id: 2, killer_name: "Player2".to_owned(), victim_name: "Player1".to_owned(), reason_name: "NONE".to_owned() },
            Quake3Events::Kill         { event_id: 7, killer_id: 1, victim_id: 2, reason_id: 1, killer_name: "Player1".to_owned(), victim_name: "Player2".to_owned(), reason_name: "NONE".to_owned() },
            Quake3Events::ShutdownGame { event_id: 8 },
        ];
        let expected_summaries = vec![
            GameMatchSummary {
                total_kills: 6,
                players: BTreeSet::from([
                    "Player1".to_owned(),
                    "Player2".to_owned(),
                ]),
                kills: BTreeMap::from([
                    ("Player1".to_owned(), 0),
                    ("Player2".to_owned(), 0),
                ]),
                game_reported_scores: None,
                disconnected_players: None,
            },
        ];
        assert_mock_summaries(events, expected_summaries)

    }

    /// Tests that if users disconnect their scores will be forgotten
    #[test]
    fn user_disconnections_zeroes_the_score() {

        // scenario: user disconnects and does not appear on the final summary
        //////////////////////////////////////////////////////////////////////
        // they will still appear in the `disconnected_players` field
        // and `total_kills` will be kept

        let events = vec![
            Quake3Events::InitGame              { event_id: 1 },
            Quake3Events::ClientConnect         { event_id: 2, client_id: 1 },
            Quake3Events::ClientUserinfoChanged { event_id: 3, client_id: 1, name: "Bartolo".to_owned() },
            Quake3Events::ClientConnect         { event_id: 4, client_id: 2 },
            Quake3Events::ClientUserinfoChanged { event_id: 5, client_id: 2, name: "Mielina".to_owned() },
            Quake3Events::Kill                  { event_id: 6, killer_id: 1, victim_id: 2, reason_id: 1, killer_name: "Bartolo".to_owned(), victim_name: "Mielina".to_owned(), reason_name: "ANY".to_owned() },
            Quake3Events::Kill                  { event_id: 7, killer_id: 2, victim_id: 1, reason_id: 2, killer_name: "Mielina".to_owned(), victim_name: "Bartolo".to_owned(), reason_name: "ANY".to_owned() },
            Quake3Events::ClientDisconnect      { event_id: 8, client_id: 1 },
            Quake3Events::ShutdownGame          { event_id: 9 },
        ];
        let expected_summaries = vec![
            GameMatchSummary {
                total_kills: 2,
                players: BTreeSet::from([
                    "Mielina".to_owned(),
                ]),
                kills: BTreeMap::from([
                    ("Mielina".to_owned(), 1),
                ]),
                game_reported_scores: None,
                disconnected_players: Some(vec![
                    (1, "Bartolo".to_owned(), 1),
                ]),
            },
        ];
        assert_mock_summaries(events, expected_summaries);


        // scenario: user reconnects in between some frags
        //////////////////////////////////////////////////
        // some frags will be deleted, they will still appear in the `disconnected_players` field
        // and `total_kills` will be kept

        let events = vec![
            Quake3Events::InitGame              { event_id: 1 },
            Quake3Events::ClientConnect         { event_id: 2,  client_id: 1 },
            Quake3Events::ClientUserinfoChanged { event_id: 3,  client_id: 1, name: "Bartolo".to_owned() },
            Quake3Events::ClientConnect         { event_id: 4,  client_id: 2 },
            Quake3Events::ClientUserinfoChanged { event_id: 5,  client_id: 2, name: "Mielina".to_owned() },
            Quake3Events::Kill                  { event_id: 6,  killer_id: 1, victim_id: 2, reason_id: 1, killer_name: "Bartolo".to_owned(), victim_name: "Mielina".to_owned(), reason_name: "ANY".to_owned() },
            Quake3Events::Kill                  { event_id: 7,  killer_id: 2, victim_id: 1, reason_id: 2, killer_name: "Mielina".to_owned(), victim_name: "Bartolo".to_owned(), reason_name: "ANY".to_owned() },
            Quake3Events::ClientDisconnect      { event_id: 8,  client_id: 1 },
            Quake3Events::ClientConnect         { event_id: 9,  client_id: 3 },
            Quake3Events::ClientUserinfoChanged { event_id: 10, client_id: 3, name: "Bartolo".to_owned() },
            Quake3Events::Kill                  { event_id: 11, killer_id: 1, victim_id: 2, reason_id: 1, killer_name: "Bartolo".to_owned(), victim_name: "Mielina".to_owned(), reason_name: "ANY".to_owned() },
            Quake3Events::Kill                  { event_id: 12, killer_id: 1, victim_id: 2, reason_id: 1, killer_name: "Bartolo".to_owned(), victim_name: "Mielina".to_owned(), reason_name: "ANY".to_owned() },
            Quake3Events::ShutdownGame          { event_id: 13 },
        ];
        let expected_summaries = vec![
            GameMatchSummary {
                total_kills: 4,
                players: BTreeSet::from([
                    "Bartolo".to_owned(),
                    "Mielina".to_owned(),
                ]),
                kills: BTreeMap::from([
                    ("Bartolo".to_owned(), 2),
                    ("Mielina".to_owned(), 1),
                ]),
                game_reported_scores: None,
                disconnected_players: Some(vec![
                    (1, "Bartolo".to_owned(), 1),
                ]),
            },
        ];
        assert_mock_summaries(events, expected_summaries)

    }

    /// Tests that user scores are correctly preserved after user nickname changes
    #[test]
    fn nick_renamings() {
        let events = vec![
            Quake3Events::InitGame              { event_id:  1 },
            Quake3Events::ClientConnect         { event_id:  2, client_id: 1 },
            Quake3Events::ClientUserinfoChanged { event_id:  3, client_id: 1, name: "Bartolo".to_owned() },
            Quake3Events::ClientConnect         { event_id:  4, client_id: 2 },
            Quake3Events::ClientUserinfoChanged { event_id:  5, client_id: 2, name: "Mielina".to_owned() },
            Quake3Events::Kill                  { event_id:  6, killer_id: 1, victim_id: 2, reason_id: 1, killer_name: "Bartolo".to_owned(), victim_name: "Mielina".to_owned(), reason_name: "ANY".to_owned() },
            Quake3Events::Kill                  { event_id:  7, killer_id: 2, victim_id: 1, reason_id: 2, killer_name: "Mielina".to_owned(), victim_name: "Bartolo".to_owned(), reason_name: "ANY".to_owned() },
            Quake3Events::ClientUserinfoChanged { event_id:  8, client_id: 1, name: "Bartholo".to_owned() },
            Quake3Events::Kill                  { event_id:  9, killer_id: 1, victim_id: 2, reason_id: 1, killer_name: "Bartholo".to_owned(), victim_name: "Mielina".to_owned(), reason_name: "ANY".to_owned() },
            Quake3Events::Kill                  { event_id: 10, killer_id: 1, victim_id: 2, reason_id: 1, killer_name: "Bartholo".to_owned(), victim_name: "Mielina".to_owned(), reason_name: "ANY".to_owned() },
            Quake3Events::ShutdownGame          { event_id: 11 },
        ];
        let expected_summaries = vec![
            GameMatchSummary {
                total_kills: 4,
                players: BTreeSet::from([
                    "Bartholo".to_owned(),
                    "Mielina".to_owned(),
                ]),
                kills: BTreeMap::from([
                    ("Bartholo".to_owned(), 3),
                    ("Mielina".to_owned(), 1),
                ]),
                game_reported_scores: None,
                disconnected_players: None,
            },
        ];
        assert_mock_summaries(events, expected_summaries)
    }


    // production use cases
    ///////////////////////
    // tests some important / controversial results from production data

    /// This set of production data shows a discrepancy between our `Kill` summary from what the game server reported on the `score` events.
    /// After a detailed inspection (see bellow), it became evident the Game Server provided "score" events is not to be trusted.
    #[test]
    fn discrepant_kills_and_scores() {
        let events = vec![
            Quake3Events::InitGame              { event_id:   1 },
            Quake3Events::ClientConnect         { event_id:   2, client_id: 2 },
            Quake3Events::ClientUserinfoChanged { event_id:   3, client_id: 2, name: "Dono da Bola".to_owned() },
            Quake3Events::ClientConnect         { event_id:   4, client_id: 3 },
            Quake3Events::ClientUserinfoChanged { event_id:   5, client_id: 3, name: "Isgalamido".to_owned() },
            Quake3Events::ClientConnect         { event_id:   6, client_id: 4 },
            Quake3Events::ClientUserinfoChanged { event_id:   7, client_id: 4, name: "Zeh".to_owned() },
            Quake3Events::Kill                  { event_id:   8, killer_id: 1022, victim_id: 3, reason_id: 22, killer_name: "<world>".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_TRIGGER_HURT".to_owned() },
            Quake3Events::Kill                  { event_id:   9, killer_id: 1022, victim_id: 2, reason_id: 19, killer_name: "<world>".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_FALLING".to_owned() },
            Quake3Events::Kill                  { event_id:  10, killer_id: 1022, victim_id: 3, reason_id: 19, killer_name: "<world>".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_FALLING".to_owned() },
            Quake3Events::Kill                  { event_id:  11, killer_id: 2, victim_id: 4, reason_id: 6, killer_name: "Dono da Bola".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            Quake3Events::Kill                  { event_id:  12, killer_id: 3, victim_id: 2, reason_id: 10, killer_name: "Isgalamido".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_RAILGUN".to_owned() },
            Quake3Events::Kill                  { event_id:  13, killer_id: 3, victim_id: 4, reason_id: 10, killer_name: "Isgalamido".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_RAILGUN".to_owned() },
            Quake3Events::Kill                  { event_id:  14, killer_id: 3, victim_id: 2, reason_id: 10, killer_name: "Isgalamido".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_RAILGUN".to_owned() },
            Quake3Events::Kill                  { event_id:  15, killer_id: 3, victim_id: 4, reason_id: 10, killer_name: "Isgalamido".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_RAILGUN".to_owned() },
            Quake3Events::Kill                  { event_id:  16, killer_id: 3, victim_id: 4, reason_id: 10, killer_name: "Isgalamido".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_RAILGUN".to_owned() },
            Quake3Events::Kill                  { event_id:  17, killer_id: 2, victim_id: 4, reason_id: 6, killer_name: "Dono da Bola".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            Quake3Events::Kill                  { event_id:  18, killer_id: 3, victim_id: 2, reason_id: 6, killer_name: "Isgalamido".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            Quake3Events::Kill                  { event_id:  19, killer_id: 1022, victim_id: 3, reason_id: 22, killer_name: "<world>".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_TRIGGER_HURT".to_owned() },
            Quake3Events::Kill                  { event_id:  20, killer_id: 4, victim_id: 2, reason_id: 6, killer_name: "Zeh".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            Quake3Events::Kill                  { event_id:  21, killer_id: 3, victim_id: 4, reason_id: 6, killer_name: "Isgalamido".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            Quake3Events::Kill                  { event_id:  22, killer_id: 3, victim_id: 4, reason_id: 7, killer_name: "Isgalamido".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill                  { event_id:  23, killer_id: 2, victim_id: 3, reason_id: 6, killer_name: "Dono da Bola".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            Quake3Events::ClientConnect         { event_id:  24, client_id: 5 },
            Quake3Events::ClientUserinfoChanged { event_id:  25, client_id: 5, name: "Assasinu Credi".to_owned() },
            Quake3Events::ClientUserinfoChanged { event_id:  26, client_id: 5, name: "Assasinu Credi".to_owned() },
            Quake3Events::Kill                  { event_id:  27, killer_id: 1022, victim_id: 2, reason_id: 19, killer_name: "<world>".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_FALLING".to_owned() },
            Quake3Events::Kill                  { event_id:  28, killer_id: 4, victim_id: 5, reason_id: 6, killer_name: "Zeh".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            Quake3Events::Kill                  { event_id:  29, killer_id: 4, victim_id: 2, reason_id: 6, killer_name: "Zeh".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            Quake3Events::Kill                  { event_id:  30, killer_id: 4, victim_id: 3, reason_id: 7, killer_name: "Zeh".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill                  { event_id:  31, killer_id: 1022, victim_id: 5, reason_id: 19, killer_name: "<world>".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_FALLING".to_owned() },
            Quake3Events::Kill                  { event_id:  32, killer_id: 1022, victim_id: 3, reason_id: 22, killer_name: "<world>".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_TRIGGER_HURT".to_owned() },
            Quake3Events::Kill                  { event_id:  33, killer_id: 4, victim_id: 2, reason_id: 7, killer_name: "Zeh".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill                  { event_id:  34, killer_id: 3, victim_id: 4, reason_id: 7, killer_name: "Isgalamido".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill                  { event_id:  35, killer_id: 3, victim_id: 5, reason_id: 3, killer_name: "Isgalamido".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_MACHINEGUN".to_owned() },
            Quake3Events::Kill                  { event_id:  36, killer_id: 3, victim_id: 4, reason_id: 3, killer_name: "Isgalamido".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_MACHINEGUN".to_owned() },
            Quake3Events::Kill                  { event_id:  37, killer_id: 2, victim_id: 3, reason_id: 6, killer_name: "Dono da Bola".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            Quake3Events::Kill                  { event_id:  38, killer_id: 2, victim_id: 2, reason_id: 7, killer_name: "Dono da Bola".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill                  { event_id:  39, killer_id: 4, victim_id: 3, reason_id: 7, killer_name: "Zeh".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill                  { event_id:  40, killer_id: 4, victim_id: 5, reason_id: 6, killer_name: "Zeh".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            Quake3Events::Kill                  { event_id:  41, killer_id: 4, victim_id: 3, reason_id: 7, killer_name: "Zeh".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill                  { event_id:  42, killer_id: 5, victim_id: 4, reason_id: 7, killer_name: "Assasinu Credi".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill                  { event_id:  43, killer_id: 1022, victim_id: 3, reason_id: 19, killer_name: "<world>".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_FALLING".to_owned() },
            Quake3Events::Kill                  { event_id:  44, killer_id: 4, victim_id: 2, reason_id: 7, killer_name: "Zeh".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill                  { event_id:  45, killer_id: 4, victim_id: 5, reason_id: 7, killer_name: "Zeh".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill                  { event_id:  46, killer_id: 3, victim_id: 2, reason_id: 7, killer_name: "Isgalamido".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill                  { event_id:  47, killer_id: 1022, victim_id: 3, reason_id: 19, killer_name: "<world>".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_FALLING".to_owned() },
            Quake3Events::Kill                  { event_id:  48, killer_id: 5, victim_id: 4, reason_id: 7, killer_name: "Assasinu Credi".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill                  { event_id:  49, killer_id: 5, victim_id: 2, reason_id: 7, killer_name: "Assasinu Credi".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill                  { event_id:  50, killer_id: 5, victim_id: 3, reason_id: 7, killer_name: "Assasinu Credi".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill                  { event_id:  51, killer_id: 2, victim_id: 3, reason_id: 6, killer_name: "Dono da Bola".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            Quake3Events::Kill                  { event_id:  52, killer_id: 1022, victim_id: 2, reason_id: 22, killer_name: "<world>".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_TRIGGER_HURT".to_owned() },
            Quake3Events::Kill                  { event_id:  53, killer_id: 4, victim_id: 5, reason_id: 7, killer_name: "Zeh".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill                  { event_id:  54, killer_id: 1022, victim_id: 2, reason_id: 22, killer_name: "<world>".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_TRIGGER_HURT".to_owned() },
            Quake3Events::Kill                  { event_id:  55, killer_id: 4, victim_id: 5, reason_id: 7, killer_name: "Zeh".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill                  { event_id:  56, killer_id: 5, victim_id: 4, reason_id: 7, killer_name: "Assasinu Credi".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill                  { event_id:  57, killer_id: 2, victim_id: 3, reason_id: 7, killer_name: "Dono da Bola".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill                  { event_id:  58, killer_id: 2, victim_id: 4, reason_id: 6, killer_name: "Dono da Bola".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            Quake3Events::Kill                  { event_id:  59, killer_id: 2, victim_id: 2, reason_id: 7, killer_name: "Dono da Bola".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill                  { event_id:  60, killer_id: 5, victim_id: 3, reason_id: 7, killer_name: "Assasinu Credi".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill                  { event_id:  61, killer_id: 1022, victim_id: 2, reason_id: 22, killer_name: "<world>".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_TRIGGER_HURT".to_owned() },
            Quake3Events::Kill                  { event_id:  62, killer_id: 1022, victim_id: 3, reason_id: 22, killer_name: "<world>".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_TRIGGER_HURT".to_owned() },
            Quake3Events::Kill                  { event_id:  63, killer_id: 4, victim_id: 2, reason_id: 6, killer_name: "Zeh".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            Quake3Events::Kill                  { event_id:  64, killer_id: 2, victim_id: 5, reason_id: 7, killer_name: "Dono da Bola".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill                  { event_id:  65, killer_id: 4, victim_id: 2, reason_id: 7, killer_name: "Zeh".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill                  { event_id:  66, killer_id: 2, victim_id: 2, reason_id: 7, killer_name: "Dono da Bola".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill                  { event_id:  67, killer_id: 4, victim_id: 3, reason_id: 7, killer_name: "Zeh".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill                  { event_id:  68, killer_id: 2, victim_id: 5, reason_id: 7, killer_name: "Dono da Bola".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill                  { event_id:  69, killer_id: 1022, victim_id: 2, reason_id: 19, killer_name: "<world>".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_FALLING".to_owned() },
            Quake3Events::Kill                  { event_id:  70, killer_id: 1022, victim_id: 4, reason_id: 19, killer_name: "<world>".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_FALLING".to_owned() },
            Quake3Events::Kill                  { event_id:  71, killer_id: 5, victim_id: 2, reason_id: 6, killer_name: "Assasinu Credi".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            Quake3Events::Kill                  { event_id:  72, killer_id: 5, victim_id: 4, reason_id: 7, killer_name: "Assasinu Credi".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill                  { event_id:  73, killer_id: 3, victim_id: 5, reason_id: 6, killer_name: "Isgalamido".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            Quake3Events::Kill                  { event_id:  74, killer_id: 2, victim_id: 5, reason_id: 7, killer_name: "Dono da Bola".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill                  { event_id:  75, killer_id: 2, victim_id: 5, reason_id: 7, killer_name: "Dono da Bola".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill                  { event_id:  76, killer_id: 4, victim_id: 3, reason_id: 7, killer_name: "Zeh".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill                  { event_id:  77, killer_id: 2, victim_id: 2, reason_id: 7, killer_name: "Dono da Bola".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill                  { event_id:  78, killer_id: 3, victim_id: 4, reason_id: 10, killer_name: "Isgalamido".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_RAILGUN".to_owned() },
            Quake3Events::Kill                  { event_id:  79, killer_id: 3, victim_id: 5, reason_id: 10, killer_name: "Isgalamido".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_RAILGUN".to_owned() },
            Quake3Events::Kill                  { event_id:  80, killer_id: 4, victim_id: 3, reason_id: 1, killer_name: "Zeh".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_SHOTGUN".to_owned() },
            Quake3Events::Kill                  { event_id:  81, killer_id: 2, victim_id: 4, reason_id: 7, killer_name: "Dono da Bola".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill                  { event_id:  82, killer_id: 4, victim_id: 5, reason_id: 7, killer_name: "Zeh".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill                  { event_id:  83, killer_id: 3, victim_id: 2, reason_id: 7, killer_name: "Isgalamido".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill                  { event_id:  84, killer_id: 4, victim_id: 5, reason_id: 6, killer_name: "Zeh".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            Quake3Events::Kill                  { event_id:  85, killer_id: 4, victim_id: 5, reason_id: 7, killer_name: "Zeh".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill                  { event_id:  86, killer_id: 3, victim_id: 4, reason_id: 7, killer_name: "Isgalamido".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill                  { event_id:  87, killer_id: 1022, victim_id: 2, reason_id: 19, killer_name: "<world>".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_FALLING".to_owned() },
            Quake3Events::Kill                  { event_id:  88, killer_id: 3, victim_id: 2, reason_id: 3, killer_name: "Isgalamido".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_MACHINEGUN".to_owned() },
            Quake3Events::Kill                  { event_id:  89, killer_id: 5, victim_id: 4, reason_id: 7, killer_name: "Assasinu Credi".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill                  { event_id:  90, killer_id: 3, victim_id: 2, reason_id: 1, killer_name: "Isgalamido".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_SHOTGUN".to_owned() },
            Quake3Events::Kill                  { event_id:  91, killer_id: 5, victim_id: 3, reason_id: 7, killer_name: "Assasinu Credi".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill                  { event_id:  92, killer_id: 2, victim_id: 5, reason_id: 6, killer_name: "Dono da Bola".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            Quake3Events::Kill                  { event_id:  93, killer_id: 2, victim_id: 4, reason_id: 6, killer_name: "Dono da Bola".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            Quake3Events::Kill                  { event_id:  94, killer_id: 5, victim_id: 2, reason_id: 7, killer_name: "Assasinu Credi".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill                  { event_id:  95, killer_id: 5, victim_id: 4, reason_id: 7, killer_name: "Assasinu Credi".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill                  { event_id:  96, killer_id: 3, victim_id: 5, reason_id: 7, killer_name: "Isgalamido".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill                  { event_id:  97, killer_id: 3, victim_id: 2, reason_id: 6, killer_name: "Isgalamido".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            Quake3Events::Kill                  { event_id:  98, killer_id: 5, victim_id: 3, reason_id: 7, killer_name: "Assasinu Credi".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill                  { event_id:  99, killer_id: 3, victim_id: 4, reason_id: 7, killer_name: "Isgalamido".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill                  { event_id: 100, killer_id: 3, victim_id: 5, reason_id: 7, killer_name: "Isgalamido".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill                  { event_id: 101, killer_id: 3, victim_id: 4, reason_id: 3, killer_name: "Isgalamido".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_MACHINEGUN".to_owned() },
            Quake3Events::Kill                  { event_id: 102, killer_id: 1022, victim_id: 5, reason_id: 22, killer_name: "<world>".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_TRIGGER_HURT".to_owned() },
            Quake3Events::Kill                  { event_id: 103, killer_id: 2, victim_id: 4, reason_id: 10, killer_name: "Dono da Bola".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_RAILGUN".to_owned() },
            Quake3Events::Kill                  { event_id: 104, killer_id: 3, victim_id: 4, reason_id: 7, killer_name: "Isgalamido".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill                  { event_id: 105, killer_id: 5, victim_id: 2, reason_id: 7, killer_name: "Assasinu Credi".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill                  { event_id: 106, killer_id: 1022, victim_id: 3, reason_id: 19, killer_name: "<world>".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_FALLING".to_owned() },
            Quake3Events::Kill                  { event_id: 107, killer_id: 2, victim_id: 5, reason_id: 7, killer_name: "Dono da Bola".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill                  { event_id: 108, killer_id: 5, victim_id: 4, reason_id: 7, killer_name: "Assasinu Credi".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill                  { event_id: 109, killer_id: 5, victim_id: 5, reason_id: 7, killer_name: "Assasinu Credi".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill                  { event_id: 110, killer_id: 3, victim_id: 2, reason_id: 7, killer_name: "Isgalamido".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill                  { event_id: 111, killer_id: 1022, victim_id: 4, reason_id: 22, killer_name: "<world>".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_TRIGGER_HURT".to_owned() },
            Quake3Events::Kill                  { event_id: 112, killer_id: 1022, victim_id: 5, reason_id: 19, killer_name: "<world>".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_FALLING".to_owned() },
            Quake3Events::Kill                  { event_id: 113, killer_id: 4, victim_id: 2, reason_id: 7, killer_name: "Zeh".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill                  { event_id: 114, killer_id: 3, victim_id: 5, reason_id: 6, killer_name: "Isgalamido".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            Quake3Events::Kill                  { event_id: 115, killer_id: 4, victim_id: 3, reason_id: 7, killer_name: "Zeh".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Exit                  { event_id: 116 },
            Quake3Events::Score                 { event_id: 117, frags: 20, client_id: 4, name: "Zeh".to_owned() },
            Quake3Events::Score                 { event_id: 118, frags: 19, client_id: 3, name: "Isgalamido".to_owned() },
            Quake3Events::Score                 { event_id: 119, frags: 11, client_id: 5, name: "Assasinu Credi".to_owned() },
            Quake3Events::Score                 { event_id: 120, frags: 5, client_id: 2, name: "Dono da Bola".to_owned() },
            Quake3Events::ShutdownGame          { event_id: 121 },
        ];
        println!("Number of kills: {}", events.iter().filter(|event| matches!(event, Quake3Events::Kill {..})).count());
        println!("Number of '<world>' kills: {}", events.iter().filter(|event| matches!(event, Quake3Events::Kill { killer_id: 1022, .. })).count());
        for player in ["Assasinu Credi", "Dono da Bola", "Isgalamido", "Zeh"] {
            let player = player.to_string();
            println!("Number of '{player}' kills: {}", events.iter().filter(|event| matches!(event, Quake3Events::Kill { killer_name, .. } if killer_name == &player)).count());
            println!("Number of 'world' kills on '{player}': {}", events.iter().filter(|event| matches!(event, Quake3Events::Kill { killer_id: 1022, victim_name, .. } if victim_name == &player)).count());
        }
        let expected_summaries = vec![
            GameMatchSummary {
                total_kills: 105,
                players: BTreeSet::from([
                    "Assasinu Credi".to_owned(),
                    "Dono da Bola".to_owned(),
                    "Isgalamido".to_owned(),
                    "Zeh".to_owned(),
                ]),
                kills: BTreeMap::from([
                    ("Assasinu Credi".to_owned(), 13),
                    ("Dono da Bola".to_owned(), 13),
                    ("Isgalamido".to_owned(), 19),
                    ("Zeh".to_owned(), 20),
                ]),
                game_reported_scores: Some(BTreeMap::from([
                    ("Assasinu Credi".to_owned(), 11),
                    ("Dono da Bola".to_owned(), 5),
                    ("Isgalamido".to_owned(), 19),
                    ("Zeh".to_owned(), 20),
                ])),
                disconnected_players: None,
            },
        ];
        assert_mock_summaries(events, expected_summaries)
    }




    // unit-integrated tests section
    ////////////////////////////////
    // the tests bellow use a real DAL implementation
    // NOTE: they were not placed under this crate's 'tests/' directory as the mentioned directory
    //       is where tests with upwards integration must reside -- tests the usage of this crate's library,
    //       whereas the following tests are for downwards integration: we are testing if the DAL implementations
    //       work with this module.

    /// The location of a log file suitable for a pedantic analysis, where all log lines should be parsed OK
    /// and the event structure must adhere 100% to the model
    const PEDANTIC_LOG_FILE_LOCATION: &str = "tests/resources/qgames_pedantic.log";


    /// Assures that big log files fully correct -- fully respecting the log syntax and the events model --
    /// can be correctly processed without generating any errors when all error detection options are enabled.
    #[test]
    fn pedantic_mode_on_pedantic_log() {
        let pedantic_config = Config {
            log_issues: true,
            stop_on_feed_errors: true,
            stop_on_event_model_violations: true,
            processor_pipeline: HashSet::from([EventAnalyserOperations::Kills, EventAnalyserOperations::PlayerIdsAndNickNamesResolutions, EventAnalyserOperations::GameReportedScores]),
        };

        let log_dao = Quake3LogFileSyncReader::new(PEDANTIC_LOG_FILE_LOCATION);
        let summaries_stream = summarize_games(Arc::new(pedantic_config), log_dao).expect("sumarize_games() shouldn't fail here");
        let summaries: Vec<GameMatchSummary> = futures::executor::block_on_stream(summaries_stream).enumerate()
            .filter_map(|(id, summary_result)| summary_result
                .map_or_else(|err| panic!("The `Stream` returned by `summarize_games()` yielded element #{id} with error: {err}"),
                             |summary| Some(summary)) )
            .collect();
        println!("{summaries:#?}");
        assert_eq!(summaries.len(), 20, "Number of game summaries don't match");
    }


    // helper functions
    ///////////////////

    fn test_config() -> Arc<Config> {
        Arc::new(Config {
            processor_pipeline: HashSet::from([
                EventAnalyserOperations::Kills,
                EventAnalyserOperations::PlayerIdsAndNickNamesResolutions,
                EventAnalyserOperations::GameReportedScores
            ]),
            ..Config::default()
        })
    }

    fn assert_mock_summaries(events: Vec<Quake3Events>, expected_summaries: Vec<GameMatchSummary>) {
        let log_dao = TestDAL::new(events);
        let summaries_stream = summarize_games(test_config(), log_dao).expect("sumarize_games() shouldn't fail here");
        let summaries: Vec<GameMatchSummary> = futures::executor::block_on_stream(summaries_stream).enumerate()
            .filter_map(|(id, summary_result)| summary_result
                .map_or_else(|err| panic!("The `Stream` returned by `summarize_games()` yielded element #{id} with error: {err}"),
                             |summary| Some(summary)) )
            .collect();
        println!("{summaries:#?}");
        assert_eq!(summaries, expected_summaries, "Summaries don't match");
    }

    fn assert_integrated_summaries(log_file_path: &str, expected_summaries: Vec<GameMatchSummary>) {
        let log_dao = Quake3LogFileSyncReader::new(log_file_path);
        let summaries_stream = summarize_games(test_config(), log_dao).expect("sumarize_games() shouldn't fail here");
        let summaries: Vec<GameMatchSummary> = futures::executor::block_on_stream(summaries_stream).enumerate()
            .filter_map(|(id, summary_result)| summary_result
                .map_or_else(|err| panic!("The `Stream` returned by `summarize_games()` yielded element #{id} with error: {err}"),
                             |summary| Some(summary)) )
            .collect();
        println!("{summaries:#?}");
        assert_eq!(summaries, expected_summaries, "Summaries don't match");
    }



    /// Mock DAL for tests
    struct TestDAL {
        events: Vec<Quake3Events>,
    }
    impl TestDAL {
        /// Creates a new mock DAL for tests, yielding the all the `events`
        pub fn new(events: Vec<Quake3Events>) -> Self {
            Self { events }
        }
    }
    impl Quake3ServerEvents for TestDAL {
        fn events_stream(self) -> Result<Pin<Box<dyn Stream<Item=Quake3Events>>>> {
            let stream = stream::iter(self.events)
                .map(|event| event);
            Ok(Box::pin(stream))
        }
    }
}