//! Builds a summary, according to the specs

use model::{
    report::GameMatchSummary,
    types::Result,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::future;
use std::pin::Pin;
use std::sync::Arc;
use futures::{Stream, stream, StreamExt};
use dal_api::Quake3ServerEvents;
use log::warn;
use quake3_server_log::model::Quake3Events;
use crate::Config;
use crate::dtos::{AnalysedEvent, BllEvent, CompositeEvent, EventModelViolations};


pub fn translate(config: Arc<Config>, log_dao: impl Quake3ServerEvents) -> Result<impl Stream<Item=CompositeEvent>> {

    let stream = log_dao.events_stream()
        .map_err(|err| format!("summarize_games(): failed at fetching the events `Stream`: {err}"))?;

    let stream = stream.enumerate()
        .inspect(move |(event_id, event_result)| if config.log_issues {
            if let Err(err) = event_result {
                warn!("Failed to process event #{event_id}: {err}");
            }
        });

    let mut in_game = false;
    let mut graceful_game_end = false;

    let stream = stream
        .map(move |(event_id, game_event_result)| {
            let event = match game_event_result {
                Ok(ref event) => event,
                Err(event_feed_err) => {
                    // // detect the end of the stream
                    // if event_id == usize::MAX {
                    //     if in_game == true {
                    //         return AnalysedEvent {
                    //             log_event: Err(Box::from(format!("Event #{}: Feed ended abruptly, in the middle of a Game", event_id+1))),
                    //             logic_event: Some(BllEvent::FeedEndedAbruptly),
                    //         }
                    //     } else {
                    //         return AnalysedEvent {
                    //             log_event: event_result,
                    //             logic_event: Some(BllEvent::FeedEndedGracefully),
                    //         }
                    //     }
                    // }
                    // // it is not the end of the stream: propagate a normal error
                    return Some(CompositeEvent::GameEvent(Err(Box::from(format!("Event #{}: Feed error: {event_feed_err}", event_id+1)))));
                },
            };

            match event {

                Quake3Events::InitGame {..} => {
                    if in_game {
                        Some(CompositeEvent::LogicEvent(BllEvent::EventModelViolation {violation: EventModelViolations::DoubleInit}))
                    } else {
                        in_game = true;
                        graceful_game_end = false;
                        Some(CompositeEvent::LogicEvent(BllEvent::NewGame))
                    }
                },

                Quake3Events::Exit => {
                    if in_game {
                        graceful_game_end = true;
                        None
                    } else {
                        Some(CompositeEvent::LogicEvent(BllEvent::EventModelViolation {violation: EventModelViolations::GameNotStarted }))
                    }
                }

                Quake3Events::ShutdownGame => {
                    if in_game {
                        if graceful_game_end {
                            Some(CompositeEvent::LogicEvent(BllEvent::GameEndedGracefully))
                        } else {
                            Some(CompositeEvent::LogicEvent(BllEvent::GameEndedManually))
                        }
                    } else {
                        Some(CompositeEvent::LogicEvent(BllEvent::EventModelViolation { violation: EventModelViolations::GameNotStarted }))
                    }
                }

                _ => Some(CompositeEvent::GameEvent(game_event_result))
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

            // game events
            let CompositeEvent::GameEvent(Ok(ref game_event)) = composite_event
                else {
                    return Some(composite_event)
                };

            match game_event {

                Quake3Events::Kill { killer_id, victim_id, reason_id, killer_name, victim_name, reason_name } => {
                    if killer_name != "<world>" {
                        Some(CompositeEvent::LogicEvent(BllEvent::IncFrags { id: *killer_id, name: killer_name.to_owned() }))
                    } else {
                        Some(CompositeEvent::LogicEvent(BllEvent::DecFrags { id: *victim_id, name: victim_name.to_owned() }))
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

            // logic events
            if let CompositeEvent::LogicEvent(ref logic_event) = composite_event {
                let react_to_nicknames_discrepancy = |id, name: String, composite_event| player_ids_and_nicks.get(&id)
                    .and_then(|stored_name| if stored_name == &Some(name.clone()) {
                        Some(composite_event)
                    } else {
                        Some(CompositeEvent::LogicEvent(
                            BllEvent::EventModelViolation {
                                violation: EventModelViolations::DiscrepantPlayerName {
                                    id,
                                    local_name: stored_name.as_ref().unwrap_or(&"NONE".to_owned()).to_owned(),
                                    game_name: name,
                                }
                            }
                        ))
                    });
                return match logic_event {
                    BllEvent::NewGame => {
                        player_ids_and_nicks.clear();
                        Some(composite_event)
                    },
                    BllEvent::IncFrags { id, name } if config.stop_on_event_model_violations => react_to_nicknames_discrepancy(*id, name.to_owned(), composite_event),
                    BllEvent::DecFrags { id, name } if config.stop_on_event_model_violations => react_to_nicknames_discrepancy(*id, name.to_owned(), composite_event),
                    _ => Some(composite_event)
                }
            }

            // game events
            let CompositeEvent::GameEvent(Ok(ref game_event)) = composite_event
                else {
                    return Some(composite_event)
                };
            match game_event {

                Quake3Events::ClientConnect { id } => {
                    player_ids_and_nicks.insert(*id, None)
                        .map_or_else(|| None,
                                     //|old_nick| Some(Err(Box::from(format!("Event #{}: Two `ClientConnect {{id: {id}}}` events received before a `ClientDisconnect` -- '{old_nick}' was there already", event_id+1)))))
                                     |old_nick| Some(CompositeEvent::LogicEvent(BllEvent::EventModelViolation {violation: EventModelViolations::DoubleConnect})))
                },

                Quake3Events::ClientUserinfoChanged { id, name: new_name } => {
                    player_ids_and_nicks.get_mut(&id)
                        //.map_or_else(|| Some(Err(Box::from(format!("Event #{}: `ClientUserinfoChanged` event received before a `ClientConnect`", event_id+1)))),
                        .map_or_else(|| Some(CompositeEvent::LogicEvent(BllEvent::EventModelViolation {violation: EventModelViolations::ClientNotConnected {id: *id, name: new_name.to_owned()}})),
                                     |old_name| old_name.replace(new_name.clone())
                                             .and_then(|old_name| Some(CompositeEvent::LogicEvent(BllEvent::RenamePlayer { id: *id, old_name, new_name: new_name.clone() })) )
                                             .or_else(|| Some(CompositeEvent::LogicEvent(BllEvent::AddPlayer { id: 0, name: new_name.to_owned() })) ) )
                },

                Quake3Events::ClientDisconnect { id } => {
                    player_ids_and_nicks.remove(id)
                        .and_then(|name| Some(CompositeEvent::LogicEvent(BllEvent::DeletePlayer {id: *id, name: name.unwrap_or("NONE".to_owned())})))
                        .or_else(|| Some(CompositeEvent::LogicEvent(BllEvent::EventModelViolation {violation: EventModelViolations::ClientNotConnected {id: *id, name: "<unknown>".to_owned()}})))
                }

                Quake3Events::Kill { killer_id, victim_id, reason_id, killer_name, victim_name, reason_name } => {
                    if killer_name != "<world>" {
                        Some(CompositeEvent::LogicEvent(BllEvent::IncFrags { id: *killer_id, name: killer_name.to_owned() }))
                    } else {
                        Some(CompositeEvent::LogicEvent(BllEvent::DecFrags { id: *victim_id, name: victim_name.to_owned() }))
                    }
                },

                _ => Some(composite_event)
            }
        })
        .filter_map(|composite_event_option| future::ready(composite_event_option))

}

/// Ties together the Logic Events in the operated `stream` into a [GameMatchSummary] ready to be presented to the user,
/// ignoring any unprocessed Game Events that might be left (due to a modest chaining of operations)
pub fn summarize(config: Arc<Config>, stream: impl Stream<Item=CompositeEvent>) -> impl Stream<Item=Result<GameMatchSummary>> {

    let mut current_game_summary = None;


    stream
        .map(move |composite_event| {

            // process only logic events
            if let CompositeEvent::LogicEvent(ref logic_event) = composite_event {
                match logic_event {

                    BllEvent::NewGame => {
                        current_game_summary
                            .replace(GameMatchSummary {
                                total_kills: 0,
                                players: BTreeSet::new(),
                                kills: BTreeMap::new(),
                                scores: None,
                                disconnected_players: None,
                            })
                            .map_or_else(|| None,
                                         |previous| Some(Err(Box::from(format!("Event #{{event_id+1}}: Two `InitGame` events received before a `ShutdownGame`"/*, event_id+1*/)))) )
                    },

                    BllEvent::AddPlayer { id, name } =>
                        if let Some(ref mut current_game_summary) = current_game_summary {
                            (!current_game_summary.players.insert(name.to_owned()))
                                .then(|| Err(Box::from(format!("Event #{{event_id+1}}: Player id: {id}, name: {name:?} is already registered"/*, event_id+1*/))))
                        } else {
                            // err: no new game event
                            None
                        },

                    BllEvent::RenamePlayer { id, old_name, new_name } =>
                        if let Some(ref mut current_game_summary) = current_game_summary {
                            current_game_summary.players.remove(old_name);
                            current_game_summary.players.insert(new_name.clone());
                            current_game_summary.kills.remove(old_name)
                                .and_then(|frags| current_game_summary.kills.insert(new_name.clone(), frags));
                            None
                        } else {
                            // err: no new game event
                            None
                        },

                    BllEvent::DeletePlayer { id, name } =>
                        if let Some(ref mut current_game_summary) = current_game_summary {
                            current_game_summary.kills.remove(name)
                                .map(|frags| current_game_summary.disconnected_players.get_or_insert_with(|| Vec::new())
                                    .push( (*id, name.clone(), frags) ));
                            (!current_game_summary.players.remove(name))
                                .then(|| Err(Box::from(format!("Event #{{event_id+1}}: Player id: {id}, name: {name:?} was not registered"/*, event_id+1*/))))
                        } else {
                            // err: no new game event
                            None
                        },

                    BllEvent::IncFrags { id, name } =>
                        if let Some(ref mut current_game_summary) = current_game_summary {
                            current_game_summary.total_kills += 1;
                            current_game_summary.players.insert(name.to_owned());
                            current_game_summary.kills.entry(name.to_owned())
                                .and_modify(|frags| *frags += 1)
                                .or_insert(1);
                            None
                        } else {
                            // err: no new game event
                            None
                        },

                    BllEvent::DecFrags { id, name } =>
                        if let Some(ref mut current_game_summary) = current_game_summary {
                            current_game_summary.total_kills += 1;
                            current_game_summary.players.insert(name.to_owned());
                            current_game_summary.kills.entry(name.to_owned())
                                .and_modify(|frags| *frags -= 1)
                                .or_insert(-1);
                            None
                        } else {
                            // err: no new game event
                            None
                        },

                    BllEvent::GameEndedManually =>
                        Some(current_game_summary.take()
                            .ok_or_else(|| Box::from(format!("err: no new game event"))) ),

                    BllEvent::GameEndedGracefully =>
                        Some(current_game_summary.take()
                            .ok_or_else(|| Box::from(format!("err: no new game event"))) ),

                    _ => None
                }
            } else {
                // ignore any remaining Game Events
                None
            }

        })
        .filter_map(|composite_event_option| future::ready(composite_event_option))
}

pub fn summarize_games(log_dao: impl Quake3ServerEvents, log_failures: bool) -> Result<impl Stream<Item=Result<GameMatchSummary>>> {
    let config = Arc::new(Config::default());
    let stream = translate(config.clone(), log_dao)?;
    let stream = kills(config.clone(), stream);
    let stream = player_ids_and_nicknames_resolutions(config.clone(), stream);
    let stream = summarize(config.clone(), stream);
    Ok(stream)
}

pub fn _summarize_games(log_dao: impl Quake3ServerEvents, log_failures: bool) -> Result<impl Stream<Item=Result<GameMatchSummary>>> {

    let stream = log_dao.events_stream()
        .map_err(|err| format!("summarize_games(): failed at fetching the events `Stream`: {err}"))?;

    let stream = stream.enumerate()
        .inspect(move |(event_id, event_result)| if log_failures {
            if let Err(err) = event_result {
                warn!("Failed to process event #{event_id}: {err}");
            }
        });

    let mut current_game_summary = None;
    let mut player_ids_and_nicks = HashMap::<u32, String>::new();

    let stream = stream
        .map(move |(event_id, event_result)| {
            let event = match event_result {
                Ok(event) => event,
                Err(event_feed_err) => return Some(Err(Box::from(format!("Event #{}: Feed error: {event_feed_err}", event_id+1)))),
            };
            match event {
                Quake3Events::InitGame { .. } => {
                    player_ids_and_nicks.clear();
                    current_game_summary
                        .replace(GameMatchSummary {
                            total_kills: 0,
                            players: BTreeSet::new(),
                            kills: BTreeMap::new(),
                            scores: None,
                            disconnected_players: None,
                        })
                        .map_or_else(|| None,
                                     |previous| Some(Err(Box::from(format!("Event #{}: Two `InitGame` events received before a `ShutdownGame`", event_id+1)))) )
                },

                Quake3Events::ClientConnect { id } => {
                    player_ids_and_nicks.insert(id, format!("Player {id}"))
                        .map_or_else(|| None,
                                     |old_nick| Some(Err(Box::from(format!("Event #{}: Two `ClientConnect {{id: {id}}}` events received before a `ClientDisconnect` -- '{old_nick}' was there already", event_id+1)))))
                },

                Quake3Events::ClientUserinfoChanged { id, name: new_nick } => {
                    let Some(ref mut current_game_summary) = current_game_summary
                        else {
                            return Some(Err(Box::from(format!("Event #{}: `ClientUserinfoChanged {{id: {id}, name: {new_nick:?}}}` event received before `InitGame`", event_id+1))))
                        };
                    player_ids_and_nicks.get_mut(&id)
                        .map_or_else(|| Some(Err(Box::from(format!("Event #{}: `ClientUserinfoChanged` event received before a `ClientConnect`", event_id+1)))),
                                     |nick| {
                                         current_game_summary.players.remove(nick);
                                         current_game_summary.players.insert(new_nick.clone());
                                         current_game_summary.kills.remove(nick)
                                             .and_then(|frags| current_game_summary.kills.insert(new_nick.clone(), frags));
                                         nick.clear();
                                         nick.push_str(&new_nick);
                                         None
                                     })

                },

                Quake3Events::ClientDisconnect { id } => {
                    let Some(ref mut current_game_summary) = current_game_summary
                        else {
                            return Some(Err(Box::from(format!("Event #{}: `ClientDisconnect` event received before `InitGame`", event_id+1))))
                        };
                    player_ids_and_nicks.remove(&id)
                        .map_or_else(|| Some(Err(Box::from(format!("Event #{}: `ClientDisconnect` event received before `ClientConnect`", event_id+1)))),
                                     |nick| {
                                         current_game_summary.players.remove(&nick);
                                         if let Some(frags) = current_game_summary.kills.remove(&nick) {
                                             current_game_summary.disconnected_players.get_or_insert_with(|| Vec::new())
                                                 .push((id, nick, frags));
                                         }
                                         None
                                     }
                        )
                },

                Quake3Events::Kill { killer_id, victim_id, reason_id, killer_name, victim_name, reason_name } => {
                    let Some(ref mut current_game_summary) = current_game_summary
                        else {
                            return Some(Err(Box::from(format!("Event #{}: `Kill` event received before `InitGame`", event_id+1))))
                        };
                    if killer_name != "<world>" {
                        current_game_summary.players.insert(killer_name.clone());
                        current_game_summary.kills.entry(killer_name)
                            .and_modify(|frags| *frags += 1)
                            .or_insert(1);
                    } else {
                        current_game_summary.kills.entry(victim_name.clone())
                            .and_modify(|frags| *frags -= 1)
                            .or_insert(-1);
                    }
                    current_game_summary.players.insert(victim_name);
                    current_game_summary.total_kills += 1;
                    None
                },
                Quake3Events::Score { frags, id, name } => {
                    let Some(ref mut current_game_summary) = current_game_summary
                        else {
                            return Some(Err(Box::from(format!("Event #{}: `Score` event received before `InitGame`", event_id+1))))
                        };
                    current_game_summary.scores.get_or_insert_with(|| BTreeMap::new())
                        .insert(name, frags);
                    None
                },
                Quake3Events::ShutdownGame => {
                    current_game_summary.take()
                        .map_or_else(|| Some(Err(Box::from(format!("Event #{}: `ShutdownGame` event received before `InitGame`", event_id+1)))),
                                     |current_game_summary| Some(Ok(current_game_summary)))
                },
                Quake3Events::Say => None,
                Quake3Events::Exit => None,
                Quake3Events::CaptureTheFlagResults { .. } => None,
                Quake3Events::ClientBegin { .. } => None,
                Quake3Events::Item => None,
                Quake3Events::Comment => None,
            }
        })
        .filter_map(|event| future::ready(event));
    Ok(stream)
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
    fn happy_path() {
        let events = vec![
            Quake3Events::InitGame { frag_limit: None, capture_limit: None, time_limit_min: None },
            Quake3Events::Kill { killer_id: 1, victim_id: 2, reason_id: 1, killer_name: "Player1".to_owned(), victim_name: "Player2".to_owned(), reason_name: "NONE".to_owned() },
            Quake3Events::Kill { killer_id: 2, victim_id: 1, reason_id: 2, killer_name: "Player2".to_owned(), victim_name: "Player1".to_owned(), reason_name: "NONE".to_owned() },
            Quake3Events::ShutdownGame,
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
                scores: None,
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
            Quake3Events::InitGame { frag_limit: None, capture_limit: None, time_limit_min: None },
            Quake3Events::Kill { killer_id: 1022, victim_id: 2, reason_id: 1, killer_name: "<world>".to_owned(), victim_name: "Player2".to_owned(), reason_name: "NONE".to_owned() },
            Quake3Events::Kill { killer_id: 2022, victim_id: 1, reason_id: 2, killer_name: "<world>".to_owned(), victim_name: "Player1".to_owned(), reason_name: "NONE".to_owned() },
            Quake3Events::Kill { killer_id: 2022, victim_id: 1, reason_id: 2, killer_name: "<world>".to_owned(), victim_name: "Player1".to_owned(), reason_name: "NONE".to_owned() },
            Quake3Events::ShutdownGame,
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
                scores: None,
                disconnected_players: None,
            },
        ];
        assert_mock_summaries(events, expected_summaries);

        // scenario: positives and negatives for a zero net result
        //////////////////////////////////////////////////////////

        let events = vec![
            Quake3Events::InitGame { frag_limit: None, capture_limit: None, time_limit_min: None },
            Quake3Events::Kill { killer_id: 1022, victim_id: 2, reason_id: 1, killer_name: "<world>".to_owned(), victim_name: "Player2".to_owned(), reason_name: "NONE".to_owned() },
            Quake3Events::Kill { killer_id: 2022, victim_id: 1, reason_id: 2, killer_name: "<world>".to_owned(), victim_name: "Player1".to_owned(), reason_name: "NONE".to_owned() },
            Quake3Events::Kill { killer_id: 2022, victim_id: 1, reason_id: 2, killer_name: "<world>".to_owned(), victim_name: "Player1".to_owned(), reason_name: "NONE".to_owned() },
            Quake3Events::Kill { killer_id: 1, victim_id: 2, reason_id: 1, killer_name: "Player1".to_owned(), victim_name: "Player2".to_owned(), reason_name: "NONE".to_owned() },
            Quake3Events::Kill { killer_id: 2, victim_id: 1, reason_id: 2, killer_name: "Player2".to_owned(), victim_name: "Player1".to_owned(), reason_name: "NONE".to_owned() },
            Quake3Events::Kill { killer_id: 1, victim_id: 2, reason_id: 1, killer_name: "Player1".to_owned(), victim_name: "Player2".to_owned(), reason_name: "NONE".to_owned() },
            Quake3Events::ShutdownGame,
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
                scores: None,
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
            Quake3Events::InitGame { frag_limit: None, capture_limit: None, time_limit_min: None },
            Quake3Events::ClientConnect { id: 1 },
            Quake3Events::ClientUserinfoChanged { id: 1, name: "Bartolo".to_owned() },
            Quake3Events::ClientBegin { id: 1 },
            Quake3Events::ClientConnect { id: 2 },
            Quake3Events::ClientUserinfoChanged { id: 2, name: "Mielina".to_owned() },
            Quake3Events::ClientBegin { id: 2 },
            Quake3Events::Kill { killer_id: 1, victim_id: 2, reason_id: 1, killer_name: "Bartolo".to_owned(), victim_name: "Mielina".to_owned(), reason_name: "ANY".to_owned() },
            Quake3Events::Kill { killer_id: 2, victim_id: 1, reason_id: 2, killer_name: "Mielina".to_owned(), victim_name: "Bartolo".to_owned(), reason_name: "ANY".to_owned() },
            Quake3Events::ClientDisconnect { id: 1 },
            Quake3Events::ShutdownGame,
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
                scores: None,
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
            Quake3Events::InitGame { frag_limit: None, capture_limit: None, time_limit_min: None },
            Quake3Events::ClientConnect { id: 1 },
            Quake3Events::ClientUserinfoChanged { id: 1, name: "Bartolo".to_owned() },
            Quake3Events::ClientBegin { id: 1 },
            Quake3Events::ClientConnect { id: 2 },
            Quake3Events::ClientUserinfoChanged { id: 2, name: "Mielina".to_owned() },
            Quake3Events::ClientBegin { id: 2 },
            Quake3Events::Kill { killer_id: 1, victim_id: 2, reason_id: 1, killer_name: "Bartolo".to_owned(), victim_name: "Mielina".to_owned(), reason_name: "ANY".to_owned() },
            Quake3Events::Kill { killer_id: 2, victim_id: 1, reason_id: 2, killer_name: "Mielina".to_owned(), victim_name: "Bartolo".to_owned(), reason_name: "ANY".to_owned() },
            Quake3Events::ClientDisconnect { id: 1 },
            Quake3Events::ClientConnect { id: 3 },
            Quake3Events::ClientUserinfoChanged { id: 3, name: "Bartolo".to_owned() },
            Quake3Events::ClientBegin { id: 3 },
            Quake3Events::Kill { killer_id: 1, victim_id: 2, reason_id: 1, killer_name: "Bartolo".to_owned(), victim_name: "Mielina".to_owned(), reason_name: "ANY".to_owned() },
            Quake3Events::Kill { killer_id: 1, victim_id: 2, reason_id: 1, killer_name: "Bartolo".to_owned(), victim_name: "Mielina".to_owned(), reason_name: "ANY".to_owned() },
            Quake3Events::ShutdownGame,
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
                scores: None,
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
            Quake3Events::InitGame { frag_limit: None, capture_limit: None, time_limit_min: None },
            Quake3Events::ClientConnect { id: 1 },
            Quake3Events::ClientUserinfoChanged { id: 1, name: "Bartolo".to_owned() },
            Quake3Events::ClientBegin { id: 1 },
            Quake3Events::ClientConnect { id: 2 },
            Quake3Events::ClientUserinfoChanged { id: 2, name: "Mielina".to_owned() },
            Quake3Events::ClientBegin { id: 2 },
            Quake3Events::Kill { killer_id: 1, victim_id: 2, reason_id: 1, killer_name: "Bartolo".to_owned(), victim_name: "Mielina".to_owned(), reason_name: "ANY".to_owned() },
            Quake3Events::Kill { killer_id: 2, victim_id: 1, reason_id: 2, killer_name: "Mielina".to_owned(), victim_name: "Bartolo".to_owned(), reason_name: "ANY".to_owned() },
            Quake3Events::ClientUserinfoChanged { id: 1, name: "Bartholo".to_owned() },
            Quake3Events::Kill { killer_id: 1, victim_id: 2, reason_id: 1, killer_name: "Bartholo".to_owned(), victim_name: "Mielina".to_owned(), reason_name: "ANY".to_owned() },
            Quake3Events::Kill { killer_id: 1, victim_id: 2, reason_id: 1, killer_name: "Bartholo".to_owned(), victim_name: "Mielina".to_owned(), reason_name: "ANY".to_owned() },
            Quake3Events::ShutdownGame,
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
                scores: None,
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
            Quake3Events::InitGame { frag_limit: Some(20), capture_limit: Some(8), time_limit_min: Some(15) },
            Quake3Events::ClientConnect { id: 2 },
            Quake3Events::ClientUserinfoChanged { id: 2, name: "Dono da Bola".to_owned() },
            Quake3Events::ClientBegin { id: 2 },
            Quake3Events::ClientConnect { id: 3 },
            Quake3Events::ClientUserinfoChanged { id: 3, name: "Isgalamido".to_owned() },
            Quake3Events::ClientBegin { id: 3 },
            Quake3Events::ClientConnect { id: 4 },
            Quake3Events::ClientUserinfoChanged { id: 4, name: "Zeh".to_owned() },
            Quake3Events::ClientBegin { id: 4 },
            Quake3Events::Kill { killer_id: 1022, victim_id: 3, reason_id: 22, killer_name: "<world>".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_TRIGGER_HURT".to_owned() },
            Quake3Events::Kill { killer_id: 1022, victim_id: 2, reason_id: 19, killer_name: "<world>".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_FALLING".to_owned() },
            Quake3Events::Kill { killer_id: 1022, victim_id: 3, reason_id: 19, killer_name: "<world>".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_FALLING".to_owned() },
            Quake3Events::Kill { killer_id: 2, victim_id: 4, reason_id: 6, killer_name: "Dono da Bola".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            Quake3Events::Kill { killer_id: 3, victim_id: 2, reason_id: 10, killer_name: "Isgalamido".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_RAILGUN".to_owned() },
            Quake3Events::Kill { killer_id: 3, victim_id: 4, reason_id: 10, killer_name: "Isgalamido".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_RAILGUN".to_owned() },
            Quake3Events::Kill { killer_id: 3, victim_id: 2, reason_id: 10, killer_name: "Isgalamido".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_RAILGUN".to_owned() },
            Quake3Events::Kill { killer_id: 3, victim_id: 4, reason_id: 10, killer_name: "Isgalamido".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_RAILGUN".to_owned() },
            Quake3Events::Kill { killer_id: 3, victim_id: 4, reason_id: 10, killer_name: "Isgalamido".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_RAILGUN".to_owned() },
            Quake3Events::Kill { killer_id: 2, victim_id: 4, reason_id: 6, killer_name: "Dono da Bola".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            Quake3Events::Kill { killer_id: 3, victim_id: 2, reason_id: 6, killer_name: "Isgalamido".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            Quake3Events::Kill { killer_id: 1022, victim_id: 3, reason_id: 22, killer_name: "<world>".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_TRIGGER_HURT".to_owned() },
            Quake3Events::Kill { killer_id: 4, victim_id: 2, reason_id: 6, killer_name: "Zeh".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            Quake3Events::Kill { killer_id: 3, victim_id: 4, reason_id: 6, killer_name: "Isgalamido".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            Quake3Events::Kill { killer_id: 3, victim_id: 4, reason_id: 7, killer_name: "Isgalamido".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill { killer_id: 2, victim_id: 3, reason_id: 6, killer_name: "Dono da Bola".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            Quake3Events::ClientConnect { id: 5 },
            Quake3Events::ClientUserinfoChanged { id: 5, name: "Assasinu Credi".to_owned() },
            Quake3Events::ClientUserinfoChanged { id: 5, name: "Assasinu Credi".to_owned() },
            Quake3Events::ClientBegin { id: 5 },
            Quake3Events::Kill { killer_id: 1022, victim_id: 2, reason_id: 19, killer_name: "<world>".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_FALLING".to_owned() },
            Quake3Events::Kill { killer_id: 4, victim_id: 5, reason_id: 6, killer_name: "Zeh".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            Quake3Events::Kill { killer_id: 4, victim_id: 2, reason_id: 6, killer_name: "Zeh".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            Quake3Events::Kill { killer_id: 4, victim_id: 3, reason_id: 7, killer_name: "Zeh".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill { killer_id: 1022, victim_id: 5, reason_id: 19, killer_name: "<world>".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_FALLING".to_owned() },
            Quake3Events::Kill { killer_id: 1022, victim_id: 3, reason_id: 22, killer_name: "<world>".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_TRIGGER_HURT".to_owned() },
            Quake3Events::Kill { killer_id: 4, victim_id: 2, reason_id: 7, killer_name: "Zeh".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill { killer_id: 3, victim_id: 4, reason_id: 7, killer_name: "Isgalamido".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill { killer_id: 3, victim_id: 5, reason_id: 3, killer_name: "Isgalamido".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_MACHINEGUN".to_owned() },
            Quake3Events::Kill { killer_id: 3, victim_id: 4, reason_id: 3, killer_name: "Isgalamido".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_MACHINEGUN".to_owned() },
            Quake3Events::Kill { killer_id: 2, victim_id: 3, reason_id: 6, killer_name: "Dono da Bola".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            Quake3Events::Kill { killer_id: 2, victim_id: 2, reason_id: 7, killer_name: "Dono da Bola".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill { killer_id: 4, victim_id: 3, reason_id: 7, killer_name: "Zeh".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill { killer_id: 4, victim_id: 5, reason_id: 6, killer_name: "Zeh".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            Quake3Events::Kill { killer_id: 4, victim_id: 3, reason_id: 7, killer_name: "Zeh".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill { killer_id: 5, victim_id: 4, reason_id: 7, killer_name: "Assasinu Credi".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill { killer_id: 1022, victim_id: 3, reason_id: 19, killer_name: "<world>".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_FALLING".to_owned() },
            Quake3Events::Kill { killer_id: 4, victim_id: 2, reason_id: 7, killer_name: "Zeh".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill { killer_id: 4, victim_id: 5, reason_id: 7, killer_name: "Zeh".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill { killer_id: 3, victim_id: 2, reason_id: 7, killer_name: "Isgalamido".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill { killer_id: 1022, victim_id: 3, reason_id: 19, killer_name: "<world>".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_FALLING".to_owned() },
            Quake3Events::Kill { killer_id: 5, victim_id: 4, reason_id: 7, killer_name: "Assasinu Credi".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill { killer_id: 5, victim_id: 2, reason_id: 7, killer_name: "Assasinu Credi".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill { killer_id: 5, victim_id: 3, reason_id: 7, killer_name: "Assasinu Credi".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill { killer_id: 2, victim_id: 3, reason_id: 6, killer_name: "Dono da Bola".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            Quake3Events::Kill { killer_id: 1022, victim_id: 2, reason_id: 22, killer_name: "<world>".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_TRIGGER_HURT".to_owned() },
            Quake3Events::Kill { killer_id: 4, victim_id: 5, reason_id: 7, killer_name: "Zeh".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill { killer_id: 1022, victim_id: 2, reason_id: 22, killer_name: "<world>".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_TRIGGER_HURT".to_owned() },
            Quake3Events::Kill { killer_id: 4, victim_id: 5, reason_id: 7, killer_name: "Zeh".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill { killer_id: 5, victim_id: 4, reason_id: 7, killer_name: "Assasinu Credi".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill { killer_id: 2, victim_id: 3, reason_id: 7, killer_name: "Dono da Bola".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill { killer_id: 2, victim_id: 4, reason_id: 6, killer_name: "Dono da Bola".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            Quake3Events::Kill { killer_id: 2, victim_id: 2, reason_id: 7, killer_name: "Dono da Bola".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill { killer_id: 5, victim_id: 3, reason_id: 7, killer_name: "Assasinu Credi".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill { killer_id: 1022, victim_id: 2, reason_id: 22, killer_name: "<world>".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_TRIGGER_HURT".to_owned() },
            Quake3Events::Kill { killer_id: 1022, victim_id: 3, reason_id: 22, killer_name: "<world>".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_TRIGGER_HURT".to_owned() },
            Quake3Events::Kill { killer_id: 4, victim_id: 2, reason_id: 6, killer_name: "Zeh".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            Quake3Events::Kill { killer_id: 2, victim_id: 5, reason_id: 7, killer_name: "Dono da Bola".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill { killer_id: 4, victim_id: 2, reason_id: 7, killer_name: "Zeh".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill { killer_id: 2, victim_id: 2, reason_id: 7, killer_name: "Dono da Bola".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill { killer_id: 4, victim_id: 3, reason_id: 7, killer_name: "Zeh".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill { killer_id: 2, victim_id: 5, reason_id: 7, killer_name: "Dono da Bola".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill { killer_id: 1022, victim_id: 2, reason_id: 19, killer_name: "<world>".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_FALLING".to_owned() },
            Quake3Events::Kill { killer_id: 1022, victim_id: 4, reason_id: 19, killer_name: "<world>".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_FALLING".to_owned() },
            Quake3Events::Kill { killer_id: 5, victim_id: 2, reason_id: 6, killer_name: "Assasinu Credi".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            Quake3Events::Kill { killer_id: 5, victim_id: 4, reason_id: 7, killer_name: "Assasinu Credi".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill { killer_id: 3, victim_id: 5, reason_id: 6, killer_name: "Isgalamido".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            Quake3Events::Kill { killer_id: 2, victim_id: 5, reason_id: 7, killer_name: "Dono da Bola".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill { killer_id: 2, victim_id: 5, reason_id: 7, killer_name: "Dono da Bola".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill { killer_id: 4, victim_id: 3, reason_id: 7, killer_name: "Zeh".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill { killer_id: 2, victim_id: 2, reason_id: 7, killer_name: "Dono da Bola".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill { killer_id: 3, victim_id: 4, reason_id: 10, killer_name: "Isgalamido".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_RAILGUN".to_owned() },
            Quake3Events::Kill { killer_id: 3, victim_id: 5, reason_id: 10, killer_name: "Isgalamido".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_RAILGUN".to_owned() },
            Quake3Events::Kill { killer_id: 4, victim_id: 3, reason_id: 1, killer_name: "Zeh".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_SHOTGUN".to_owned() },
            Quake3Events::Kill { killer_id: 2, victim_id: 4, reason_id: 7, killer_name: "Dono da Bola".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill { killer_id: 4, victim_id: 5, reason_id: 7, killer_name: "Zeh".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill { killer_id: 3, victim_id: 2, reason_id: 7, killer_name: "Isgalamido".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill { killer_id: 4, victim_id: 5, reason_id: 6, killer_name: "Zeh".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            Quake3Events::Kill { killer_id: 4, victim_id: 5, reason_id: 7, killer_name: "Zeh".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill { killer_id: 3, victim_id: 4, reason_id: 7, killer_name: "Isgalamido".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill { killer_id: 1022, victim_id: 2, reason_id: 19, killer_name: "<world>".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_FALLING".to_owned() },
            Quake3Events::Kill { killer_id: 3, victim_id: 2, reason_id: 3, killer_name: "Isgalamido".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_MACHINEGUN".to_owned() },
            Quake3Events::Kill { killer_id: 5, victim_id: 4, reason_id: 7, killer_name: "Assasinu Credi".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill { killer_id: 3, victim_id: 2, reason_id: 1, killer_name: "Isgalamido".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_SHOTGUN".to_owned() },
            Quake3Events::Kill { killer_id: 5, victim_id: 3, reason_id: 7, killer_name: "Assasinu Credi".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill { killer_id: 2, victim_id: 5, reason_id: 6, killer_name: "Dono da Bola".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            Quake3Events::Kill { killer_id: 2, victim_id: 4, reason_id: 6, killer_name: "Dono da Bola".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            Quake3Events::Kill { killer_id: 5, victim_id: 2, reason_id: 7, killer_name: "Assasinu Credi".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill { killer_id: 5, victim_id: 4, reason_id: 7, killer_name: "Assasinu Credi".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill { killer_id: 3, victim_id: 5, reason_id: 7, killer_name: "Isgalamido".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill { killer_id: 3, victim_id: 2, reason_id: 6, killer_name: "Isgalamido".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            Quake3Events::Kill { killer_id: 5, victim_id: 3, reason_id: 7, killer_name: "Assasinu Credi".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill { killer_id: 3, victim_id: 4, reason_id: 7, killer_name: "Isgalamido".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill { killer_id: 3, victim_id: 5, reason_id: 7, killer_name: "Isgalamido".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill { killer_id: 3, victim_id: 4, reason_id: 3, killer_name: "Isgalamido".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_MACHINEGUN".to_owned() },
            Quake3Events::Kill { killer_id: 1022, victim_id: 5, reason_id: 22, killer_name: "<world>".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_TRIGGER_HURT".to_owned() },
            Quake3Events::Kill { killer_id: 2, victim_id: 4, reason_id: 10, killer_name: "Dono da Bola".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_RAILGUN".to_owned() },
            Quake3Events::Kill { killer_id: 3, victim_id: 4, reason_id: 7, killer_name: "Isgalamido".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill { killer_id: 5, victim_id: 2, reason_id: 7, killer_name: "Assasinu Credi".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill { killer_id: 1022, victim_id: 3, reason_id: 19, killer_name: "<world>".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_FALLING".to_owned() },
            Quake3Events::Kill { killer_id: 2, victim_id: 5, reason_id: 7, killer_name: "Dono da Bola".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill { killer_id: 5, victim_id: 4, reason_id: 7, killer_name: "Assasinu Credi".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill { killer_id: 5, victim_id: 5, reason_id: 7, killer_name: "Assasinu Credi".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill { killer_id: 3, victim_id: 2, reason_id: 7, killer_name: "Isgalamido".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill { killer_id: 1022, victim_id: 4, reason_id: 22, killer_name: "<world>".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_TRIGGER_HURT".to_owned() },
            Quake3Events::Kill { killer_id: 1022, victim_id: 5, reason_id: 19, killer_name: "<world>".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_FALLING".to_owned() },
            Quake3Events::Kill { killer_id: 4, victim_id: 2, reason_id: 7, killer_name: "Zeh".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Kill { killer_id: 3, victim_id: 5, reason_id: 6, killer_name: "Isgalamido".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            Quake3Events::Kill { killer_id: 4, victim_id: 3, reason_id: 7, killer_name: "Zeh".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            Quake3Events::Exit,
            Quake3Events::Score { frags: 20, id: 4, name: "Zeh".to_owned() },
            Quake3Events::Score { frags: 19, id: 3, name: "Isgalamido".to_owned() },
            Quake3Events::Score { frags: 11, id: 5, name: "Assasinu Credi".to_owned() },
            Quake3Events::Score { frags: 5, id: 2, name: "Dono da Bola".to_owned() },
            Quake3Events::ShutdownGame,
        ];
        println!("Number of kills: {}", events.iter().filter(|event| matches!(event, Quake3Events::Kill {..})).count());
        println!("Number of '<world>' kills: {}", events.iter().filter(|event| matches!(event, Quake3Events::Kill { killer_id: 1022, .. })).count());
        for player in ["Assasinu Credi", "Dono da Bola", "Isgalamido", "Zeh"] {
            let player = player.to_string();
            println!("Number of '{player}' kills: {}", events.iter().filter(|event| matches!(event, Quake3Events::Kill { killer_name, .. } if killer_name == &player)).count());
            println!("Number of 'world' kills on '{player}': {}", events.iter().filter(|event| matches!(event, Quake3Events::Kill { killer_id: 1022, victim_name, .. } if victim_name == &player)).count());
        }
        let log_dao = TestDAL::new(events);
        let summaries_stream = summarize_games(log_dao, false).expect("sumarize_games() shouldn't fail here");
        let summaries: Vec<GameMatchSummary> = futures::executor::block_on_stream(summaries_stream).enumerate()
            .filter_map(|(id, summary_result)| summary_result
                .map_or_else(|err| panic!("The `Stream` returned by `summarize_games()` yielded element #{id} with error: {err}"),
                             |summary| Some(summary)) )
            .collect();
        println!("{summaries:#?}");
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


    #[test]
    fn fully_working_log() {
        assert_integrated_summaries(PEDANTIC_LOG_FILE_LOCATION, vec![])
    }

    #[test]
    fn _discrepant() {
        assert_integrated_summaries("tests/resources/discrepant.log", vec![])
    }


    fn assert_mock_summaries(events: Vec<Quake3Events>, expected_summaries: Vec<GameMatchSummary>) {
        let log_dao = TestDAL::new(events);
        let summaries_stream = summarize_games(log_dao, false).expect("sumarize_games() shouldn't fail here");
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
        let summaries_stream = summarize_games(log_dao, false).expect("sumarize_games() shouldn't fail here");
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
        fn events_stream(self) -> Result<Pin<Box<dyn Stream<Item=Result<Quake3Events>>>>> {
            let stream = stream::iter(self.events)
                .map(|event| Ok(event));
            Ok(Box::pin(stream))
        }
    }
}