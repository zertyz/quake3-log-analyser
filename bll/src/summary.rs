//! Builds a summary, according to the specs

use model::{
    report::GameMatchSummary,
    types::Result,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::future;
use std::pin::Pin;
use futures::{Stream, stream, StreamExt};
use dal_api::Quake3ServerEvents;
use log::warn;
use quake3_server_log::model::LogEvent;

pub fn summarize_games(log_dao: impl Quake3ServerEvents, log_failures: bool) -> Result<impl Stream<Item=Result<GameMatchSummary>>> {

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
                LogEvent::InitGame { .. } => {
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

                LogEvent::ClientConnect { id } => {
                    player_ids_and_nicks.insert(id, format!("Player {id}"))
                        .map_or_else(|| None,
                                     |old_nick| Some(Err(Box::from(format!("Event #{}: Two `ClientConnect {{id: {id}}}` events received before a `ClientDisconnect` -- '{old_nick}' was there already", event_id+1)))))
                },

                LogEvent::ClientUserinfoChanged { id, name: new_nick } => {
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

                LogEvent::ClientDisconnect { id } => {
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

                LogEvent::Kill { killer_id, victim_id, reason_id, killer_name, victim_name, reason_name } => {
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
                LogEvent::Score { frags, id, name } => {
                    let Some(ref mut current_game_summary) = current_game_summary
                        else {
                            return Some(Err(Box::from(format!("Event #{}: `Score` event received before `InitGame`", event_id+1))))
                        };
                    current_game_summary.scores.get_or_insert_with(|| BTreeMap::new())
                        .insert(name, frags);
                    None
                },
                LogEvent::ShutdownGame => {
                    current_game_summary.take()
                        .map_or_else(|| Some(Err(Box::from(format!("Event #{}: `ShutdownGame` event received before `InitGame`", event_id+1)))),
                                     |current_game_summary| Some(Ok(current_game_summary)))
                },
                LogEvent::Say => None,
                LogEvent::Exit => None,
                LogEvent::CaptureTheFlagResults { .. } => None,
                LogEvent::ClientBegin { .. } => None,
                LogEvent::Item => None,
                LogEvent::Comment => None,
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
            LogEvent::InitGame { frag_limit: None, capture_limit: None, time_limit_min: None },
            LogEvent::Kill { killer_id: 1, victim_id: 2, reason_id: 1, killer_name: "Player1".to_owned(), victim_name: "Player2".to_owned(), reason_name: "NONE".to_owned() },
            LogEvent::Kill { killer_id: 2, victim_id: 1, reason_id: 2, killer_name: "Player2".to_owned(), victim_name: "Player1".to_owned(), reason_name: "NONE".to_owned() },
            LogEvent::ShutdownGame,
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
            LogEvent::InitGame { frag_limit: None, capture_limit: None, time_limit_min: None },
            LogEvent::Kill { killer_id: 1022, victim_id: 2, reason_id: 1, killer_name: "<world>".to_owned(), victim_name: "Player2".to_owned(), reason_name: "NONE".to_owned() },
            LogEvent::Kill { killer_id: 2022, victim_id: 1, reason_id: 2, killer_name: "<world>".to_owned(), victim_name: "Player1".to_owned(), reason_name: "NONE".to_owned() },
            LogEvent::Kill { killer_id: 2022, victim_id: 1, reason_id: 2, killer_name: "<world>".to_owned(), victim_name: "Player1".to_owned(), reason_name: "NONE".to_owned() },
            LogEvent::ShutdownGame,
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
            LogEvent::InitGame { frag_limit: None, capture_limit: None, time_limit_min: None },
            LogEvent::Kill { killer_id: 1022, victim_id: 2, reason_id: 1, killer_name: "<world>".to_owned(), victim_name: "Player2".to_owned(), reason_name: "NONE".to_owned() },
            LogEvent::Kill { killer_id: 2022, victim_id: 1, reason_id: 2, killer_name: "<world>".to_owned(), victim_name: "Player1".to_owned(), reason_name: "NONE".to_owned() },
            LogEvent::Kill { killer_id: 2022, victim_id: 1, reason_id: 2, killer_name: "<world>".to_owned(), victim_name: "Player1".to_owned(), reason_name: "NONE".to_owned() },
            LogEvent::Kill { killer_id: 1, victim_id: 2, reason_id: 1, killer_name: "Player1".to_owned(), victim_name: "Player2".to_owned(), reason_name: "NONE".to_owned() },
            LogEvent::Kill { killer_id: 2, victim_id: 1, reason_id: 2, killer_name: "Player2".to_owned(), victim_name: "Player1".to_owned(), reason_name: "NONE".to_owned() },
            LogEvent::Kill { killer_id: 1, victim_id: 2, reason_id: 1, killer_name: "Player1".to_owned(), victim_name: "Player2".to_owned(), reason_name: "NONE".to_owned() },
            LogEvent::ShutdownGame,
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
            LogEvent::InitGame { frag_limit: None, capture_limit: None, time_limit_min: None },
            LogEvent::ClientConnect { id: 1 },
            LogEvent::ClientUserinfoChanged { id: 1, name: "Bartolo".to_owned() },
            LogEvent::ClientBegin { id: 1 },
            LogEvent::ClientConnect { id: 2 },
            LogEvent::ClientUserinfoChanged { id: 2, name: "Mielina".to_owned() },
            LogEvent::ClientBegin { id: 2 },
            LogEvent::Kill { killer_id: 1, victim_id: 2, reason_id: 1, killer_name: "Bartolo".to_owned(), victim_name: "Mielina".to_owned(), reason_name: "ANY".to_owned() },
            LogEvent::Kill { killer_id: 2, victim_id: 1, reason_id: 2, killer_name: "Mielina".to_owned(), victim_name: "Bartolo".to_owned(), reason_name: "ANY".to_owned() },
            LogEvent::ClientDisconnect { id: 1 },
            LogEvent::ShutdownGame,
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
            LogEvent::InitGame { frag_limit: None, capture_limit: None, time_limit_min: None },
            LogEvent::ClientConnect { id: 1 },
            LogEvent::ClientUserinfoChanged { id: 1, name: "Bartolo".to_owned() },
            LogEvent::ClientBegin { id: 1 },
            LogEvent::ClientConnect { id: 2 },
            LogEvent::ClientUserinfoChanged { id: 2, name: "Mielina".to_owned() },
            LogEvent::ClientBegin { id: 2 },
            LogEvent::Kill { killer_id: 1, victim_id: 2, reason_id: 1, killer_name: "Bartolo".to_owned(), victim_name: "Mielina".to_owned(), reason_name: "ANY".to_owned() },
            LogEvent::Kill { killer_id: 2, victim_id: 1, reason_id: 2, killer_name: "Mielina".to_owned(), victim_name: "Bartolo".to_owned(), reason_name: "ANY".to_owned() },
            LogEvent::ClientDisconnect { id: 1 },
            LogEvent::ClientConnect { id: 3 },
            LogEvent::ClientUserinfoChanged { id: 3, name: "Bartolo".to_owned() },
            LogEvent::ClientBegin { id: 3 },
            LogEvent::Kill { killer_id: 1, victim_id: 2, reason_id: 1, killer_name: "Bartolo".to_owned(), victim_name: "Mielina".to_owned(), reason_name: "ANY".to_owned() },
            LogEvent::Kill { killer_id: 1, victim_id: 2, reason_id: 1, killer_name: "Bartolo".to_owned(), victim_name: "Mielina".to_owned(), reason_name: "ANY".to_owned() },
            LogEvent::ShutdownGame,
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
            LogEvent::InitGame { frag_limit: None, capture_limit: None, time_limit_min: None },
            LogEvent::ClientConnect { id: 1 },
            LogEvent::ClientUserinfoChanged { id: 1, name: "Bartolo".to_owned() },
            LogEvent::ClientBegin { id: 1 },
            LogEvent::ClientConnect { id: 2 },
            LogEvent::ClientUserinfoChanged { id: 2, name: "Mielina".to_owned() },
            LogEvent::ClientBegin { id: 2 },
            LogEvent::Kill { killer_id: 1, victim_id: 2, reason_id: 1, killer_name: "Bartolo".to_owned(), victim_name: "Mielina".to_owned(), reason_name: "ANY".to_owned() },
            LogEvent::Kill { killer_id: 2, victim_id: 1, reason_id: 2, killer_name: "Mielina".to_owned(), victim_name: "Bartolo".to_owned(), reason_name: "ANY".to_owned() },
            LogEvent::ClientUserinfoChanged { id: 1, name: "Bartholo".to_owned() },
            LogEvent::Kill { killer_id: 1, victim_id: 2, reason_id: 1, killer_name: "Bartholo".to_owned(), victim_name: "Mielina".to_owned(), reason_name: "ANY".to_owned() },
            LogEvent::Kill { killer_id: 1, victim_id: 2, reason_id: 1, killer_name: "Bartholo".to_owned(), victim_name: "Mielina".to_owned(), reason_name: "ANY".to_owned() },
            LogEvent::ShutdownGame,
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
            LogEvent::InitGame { frag_limit: Some(20), capture_limit: Some(8), time_limit_min: Some(15) },
            LogEvent::ClientConnect { id: 2 },
            LogEvent::ClientUserinfoChanged { id: 2, name: "Dono da Bola".to_owned() },
            LogEvent::ClientBegin { id: 2 },
            LogEvent::ClientConnect { id: 3 },
            LogEvent::ClientUserinfoChanged { id: 3, name: "Isgalamido".to_owned() },
            LogEvent::ClientBegin { id: 3 },
            LogEvent::ClientConnect { id: 4 },
            LogEvent::ClientUserinfoChanged { id: 4, name: "Zeh".to_owned() },
            LogEvent::ClientBegin { id: 4 },
            LogEvent::Kill { killer_id: 1022, victim_id: 3, reason_id: 22, killer_name: "<world>".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_TRIGGER_HURT".to_owned() },
            LogEvent::Kill { killer_id: 1022, victim_id: 2, reason_id: 19, killer_name: "<world>".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_FALLING".to_owned() },
            LogEvent::Kill { killer_id: 1022, victim_id: 3, reason_id: 19, killer_name: "<world>".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_FALLING".to_owned() },
            LogEvent::Kill { killer_id: 2, victim_id: 4, reason_id: 6, killer_name: "Dono da Bola".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            LogEvent::Kill { killer_id: 3, victim_id: 2, reason_id: 10, killer_name: "Isgalamido".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_RAILGUN".to_owned() },
            LogEvent::Kill { killer_id: 3, victim_id: 4, reason_id: 10, killer_name: "Isgalamido".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_RAILGUN".to_owned() },
            LogEvent::Kill { killer_id: 3, victim_id: 2, reason_id: 10, killer_name: "Isgalamido".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_RAILGUN".to_owned() },
            LogEvent::Kill { killer_id: 3, victim_id: 4, reason_id: 10, killer_name: "Isgalamido".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_RAILGUN".to_owned() },
            LogEvent::Kill { killer_id: 3, victim_id: 4, reason_id: 10, killer_name: "Isgalamido".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_RAILGUN".to_owned() },
            LogEvent::Kill { killer_id: 2, victim_id: 4, reason_id: 6, killer_name: "Dono da Bola".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            LogEvent::Kill { killer_id: 3, victim_id: 2, reason_id: 6, killer_name: "Isgalamido".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            LogEvent::Kill { killer_id: 1022, victim_id: 3, reason_id: 22, killer_name: "<world>".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_TRIGGER_HURT".to_owned() },
            LogEvent::Kill { killer_id: 4, victim_id: 2, reason_id: 6, killer_name: "Zeh".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            LogEvent::Kill { killer_id: 3, victim_id: 4, reason_id: 6, killer_name: "Isgalamido".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            LogEvent::Kill { killer_id: 3, victim_id: 4, reason_id: 7, killer_name: "Isgalamido".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            LogEvent::Kill { killer_id: 2, victim_id: 3, reason_id: 6, killer_name: "Dono da Bola".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            LogEvent::ClientConnect { id: 5 },
            LogEvent::ClientUserinfoChanged { id: 5, name: "Assasinu Credi".to_owned() },
            LogEvent::ClientUserinfoChanged { id: 5, name: "Assasinu Credi".to_owned() },
            LogEvent::ClientBegin { id: 5 },
            LogEvent::Kill { killer_id: 1022, victim_id: 2, reason_id: 19, killer_name: "<world>".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_FALLING".to_owned() },
            LogEvent::Kill { killer_id: 4, victim_id: 5, reason_id: 6, killer_name: "Zeh".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            LogEvent::Kill { killer_id: 4, victim_id: 2, reason_id: 6, killer_name: "Zeh".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            LogEvent::Kill { killer_id: 4, victim_id: 3, reason_id: 7, killer_name: "Zeh".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            LogEvent::Kill { killer_id: 1022, victim_id: 5, reason_id: 19, killer_name: "<world>".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_FALLING".to_owned() },
            LogEvent::Kill { killer_id: 1022, victim_id: 3, reason_id: 22, killer_name: "<world>".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_TRIGGER_HURT".to_owned() },
            LogEvent::Kill { killer_id: 4, victim_id: 2, reason_id: 7, killer_name: "Zeh".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            LogEvent::Kill { killer_id: 3, victim_id: 4, reason_id: 7, killer_name: "Isgalamido".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            LogEvent::Kill { killer_id: 3, victim_id: 5, reason_id: 3, killer_name: "Isgalamido".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_MACHINEGUN".to_owned() },
            LogEvent::Kill { killer_id: 3, victim_id: 4, reason_id: 3, killer_name: "Isgalamido".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_MACHINEGUN".to_owned() },
            LogEvent::Kill { killer_id: 2, victim_id: 3, reason_id: 6, killer_name: "Dono da Bola".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            LogEvent::Kill { killer_id: 2, victim_id: 2, reason_id: 7, killer_name: "Dono da Bola".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            LogEvent::Kill { killer_id: 4, victim_id: 3, reason_id: 7, killer_name: "Zeh".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            LogEvent::Kill { killer_id: 4, victim_id: 5, reason_id: 6, killer_name: "Zeh".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            LogEvent::Kill { killer_id: 4, victim_id: 3, reason_id: 7, killer_name: "Zeh".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            LogEvent::Kill { killer_id: 5, victim_id: 4, reason_id: 7, killer_name: "Assasinu Credi".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            LogEvent::Kill { killer_id: 1022, victim_id: 3, reason_id: 19, killer_name: "<world>".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_FALLING".to_owned() },
            LogEvent::Kill { killer_id: 4, victim_id: 2, reason_id: 7, killer_name: "Zeh".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            LogEvent::Kill { killer_id: 4, victim_id: 5, reason_id: 7, killer_name: "Zeh".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            LogEvent::Kill { killer_id: 3, victim_id: 2, reason_id: 7, killer_name: "Isgalamido".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            LogEvent::Kill { killer_id: 1022, victim_id: 3, reason_id: 19, killer_name: "<world>".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_FALLING".to_owned() },
            LogEvent::Kill { killer_id: 5, victim_id: 4, reason_id: 7, killer_name: "Assasinu Credi".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            LogEvent::Kill { killer_id: 5, victim_id: 2, reason_id: 7, killer_name: "Assasinu Credi".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            LogEvent::Kill { killer_id: 5, victim_id: 3, reason_id: 7, killer_name: "Assasinu Credi".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            LogEvent::Kill { killer_id: 2, victim_id: 3, reason_id: 6, killer_name: "Dono da Bola".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            LogEvent::Kill { killer_id: 1022, victim_id: 2, reason_id: 22, killer_name: "<world>".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_TRIGGER_HURT".to_owned() },
            LogEvent::Kill { killer_id: 4, victim_id: 5, reason_id: 7, killer_name: "Zeh".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            LogEvent::Kill { killer_id: 1022, victim_id: 2, reason_id: 22, killer_name: "<world>".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_TRIGGER_HURT".to_owned() },
            LogEvent::Kill { killer_id: 4, victim_id: 5, reason_id: 7, killer_name: "Zeh".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            LogEvent::Kill { killer_id: 5, victim_id: 4, reason_id: 7, killer_name: "Assasinu Credi".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            LogEvent::Kill { killer_id: 2, victim_id: 3, reason_id: 7, killer_name: "Dono da Bola".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            LogEvent::Kill { killer_id: 2, victim_id: 4, reason_id: 6, killer_name: "Dono da Bola".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            LogEvent::Kill { killer_id: 2, victim_id: 2, reason_id: 7, killer_name: "Dono da Bola".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            LogEvent::Kill { killer_id: 5, victim_id: 3, reason_id: 7, killer_name: "Assasinu Credi".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            LogEvent::Kill { killer_id: 1022, victim_id: 2, reason_id: 22, killer_name: "<world>".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_TRIGGER_HURT".to_owned() },
            LogEvent::Kill { killer_id: 1022, victim_id: 3, reason_id: 22, killer_name: "<world>".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_TRIGGER_HURT".to_owned() },
            LogEvent::Kill { killer_id: 4, victim_id: 2, reason_id: 6, killer_name: "Zeh".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            LogEvent::Kill { killer_id: 2, victim_id: 5, reason_id: 7, killer_name: "Dono da Bola".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            LogEvent::Kill { killer_id: 4, victim_id: 2, reason_id: 7, killer_name: "Zeh".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            LogEvent::Kill { killer_id: 2, victim_id: 2, reason_id: 7, killer_name: "Dono da Bola".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            LogEvent::Kill { killer_id: 4, victim_id: 3, reason_id: 7, killer_name: "Zeh".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            LogEvent::Kill { killer_id: 2, victim_id: 5, reason_id: 7, killer_name: "Dono da Bola".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            LogEvent::Kill { killer_id: 1022, victim_id: 2, reason_id: 19, killer_name: "<world>".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_FALLING".to_owned() },
            LogEvent::Kill { killer_id: 1022, victim_id: 4, reason_id: 19, killer_name: "<world>".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_FALLING".to_owned() },
            LogEvent::Kill { killer_id: 5, victim_id: 2, reason_id: 6, killer_name: "Assasinu Credi".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            LogEvent::Kill { killer_id: 5, victim_id: 4, reason_id: 7, killer_name: "Assasinu Credi".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            LogEvent::Kill { killer_id: 3, victim_id: 5, reason_id: 6, killer_name: "Isgalamido".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            LogEvent::Kill { killer_id: 2, victim_id: 5, reason_id: 7, killer_name: "Dono da Bola".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            LogEvent::Kill { killer_id: 2, victim_id: 5, reason_id: 7, killer_name: "Dono da Bola".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            LogEvent::Kill { killer_id: 4, victim_id: 3, reason_id: 7, killer_name: "Zeh".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            LogEvent::Kill { killer_id: 2, victim_id: 2, reason_id: 7, killer_name: "Dono da Bola".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            LogEvent::Kill { killer_id: 3, victim_id: 4, reason_id: 10, killer_name: "Isgalamido".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_RAILGUN".to_owned() },
            LogEvent::Kill { killer_id: 3, victim_id: 5, reason_id: 10, killer_name: "Isgalamido".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_RAILGUN".to_owned() },
            LogEvent::Kill { killer_id: 4, victim_id: 3, reason_id: 1, killer_name: "Zeh".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_SHOTGUN".to_owned() },
            LogEvent::Kill { killer_id: 2, victim_id: 4, reason_id: 7, killer_name: "Dono da Bola".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            LogEvent::Kill { killer_id: 4, victim_id: 5, reason_id: 7, killer_name: "Zeh".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            LogEvent::Kill { killer_id: 3, victim_id: 2, reason_id: 7, killer_name: "Isgalamido".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            LogEvent::Kill { killer_id: 4, victim_id: 5, reason_id: 6, killer_name: "Zeh".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            LogEvent::Kill { killer_id: 4, victim_id: 5, reason_id: 7, killer_name: "Zeh".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            LogEvent::Kill { killer_id: 3, victim_id: 4, reason_id: 7, killer_name: "Isgalamido".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            LogEvent::Kill { killer_id: 1022, victim_id: 2, reason_id: 19, killer_name: "<world>".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_FALLING".to_owned() },
            LogEvent::Kill { killer_id: 3, victim_id: 2, reason_id: 3, killer_name: "Isgalamido".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_MACHINEGUN".to_owned() },
            LogEvent::Kill { killer_id: 5, victim_id: 4, reason_id: 7, killer_name: "Assasinu Credi".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            LogEvent::Kill { killer_id: 3, victim_id: 2, reason_id: 1, killer_name: "Isgalamido".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_SHOTGUN".to_owned() },
            LogEvent::Kill { killer_id: 5, victim_id: 3, reason_id: 7, killer_name: "Assasinu Credi".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            LogEvent::Kill { killer_id: 2, victim_id: 5, reason_id: 6, killer_name: "Dono da Bola".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            LogEvent::Kill { killer_id: 2, victim_id: 4, reason_id: 6, killer_name: "Dono da Bola".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            LogEvent::Kill { killer_id: 5, victim_id: 2, reason_id: 7, killer_name: "Assasinu Credi".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            LogEvent::Kill { killer_id: 5, victim_id: 4, reason_id: 7, killer_name: "Assasinu Credi".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            LogEvent::Kill { killer_id: 3, victim_id: 5, reason_id: 7, killer_name: "Isgalamido".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            LogEvent::Kill { killer_id: 3, victim_id: 2, reason_id: 6, killer_name: "Isgalamido".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            LogEvent::Kill { killer_id: 5, victim_id: 3, reason_id: 7, killer_name: "Assasinu Credi".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            LogEvent::Kill { killer_id: 3, victim_id: 4, reason_id: 7, killer_name: "Isgalamido".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            LogEvent::Kill { killer_id: 3, victim_id: 5, reason_id: 7, killer_name: "Isgalamido".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            LogEvent::Kill { killer_id: 3, victim_id: 4, reason_id: 3, killer_name: "Isgalamido".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_MACHINEGUN".to_owned() },
            LogEvent::Kill { killer_id: 1022, victim_id: 5, reason_id: 22, killer_name: "<world>".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_TRIGGER_HURT".to_owned() },
            LogEvent::Kill { killer_id: 2, victim_id: 4, reason_id: 10, killer_name: "Dono da Bola".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_RAILGUN".to_owned() },
            LogEvent::Kill { killer_id: 3, victim_id: 4, reason_id: 7, killer_name: "Isgalamido".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            LogEvent::Kill { killer_id: 5, victim_id: 2, reason_id: 7, killer_name: "Assasinu Credi".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            LogEvent::Kill { killer_id: 1022, victim_id: 3, reason_id: 19, killer_name: "<world>".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_FALLING".to_owned() },
            LogEvent::Kill { killer_id: 2, victim_id: 5, reason_id: 7, killer_name: "Dono da Bola".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            LogEvent::Kill { killer_id: 5, victim_id: 4, reason_id: 7, killer_name: "Assasinu Credi".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            LogEvent::Kill { killer_id: 5, victim_id: 5, reason_id: 7, killer_name: "Assasinu Credi".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            LogEvent::Kill { killer_id: 3, victim_id: 2, reason_id: 7, killer_name: "Isgalamido".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            LogEvent::Kill { killer_id: 1022, victim_id: 4, reason_id: 22, killer_name: "<world>".to_owned(), victim_name: "Zeh".to_owned(), reason_name: "MOD_TRIGGER_HURT".to_owned() },
            LogEvent::Kill { killer_id: 1022, victim_id: 5, reason_id: 19, killer_name: "<world>".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_FALLING".to_owned() },
            LogEvent::Kill { killer_id: 4, victim_id: 2, reason_id: 7, killer_name: "Zeh".to_owned(), victim_name: "Dono da Bola".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            LogEvent::Kill { killer_id: 3, victim_id: 5, reason_id: 6, killer_name: "Isgalamido".to_owned(), victim_name: "Assasinu Credi".to_owned(), reason_name: "MOD_ROCKET".to_owned() },
            LogEvent::Kill { killer_id: 4, victim_id: 3, reason_id: 7, killer_name: "Zeh".to_owned(), victim_name: "Isgalamido".to_owned(), reason_name: "MOD_ROCKET_SPLASH".to_owned() },
            LogEvent::Exit,
            LogEvent::Score { frags: 20, id: 4, name: "Zeh".to_owned() },
            LogEvent::Score { frags: 19, id: 3, name: "Isgalamido".to_owned() },
            LogEvent::Score { frags: 11, id: 5, name: "Assasinu Credi".to_owned() },
            LogEvent::Score { frags: 5, id: 2, name: "Dono da Bola".to_owned() },
            LogEvent::ShutdownGame,
        ];
        println!("Number of kills: {}", events.iter().filter(|event| matches!(event, LogEvent::Kill {..})).count());
        println!("Number of '<world>' kills: {}", events.iter().filter(|event| matches!(event, LogEvent::Kill { killer_id: 1022, .. })).count());
        for player in ["Assasinu Credi", "Dono da Bola", "Isgalamido", "Zeh"] {
            let player = player.to_string();
            println!("Number of '{player}' kills: {}", events.iter().filter(|event| matches!(event, LogEvent::Kill { killer_name, .. } if killer_name == &player)).count());
            println!("Number of 'world' kills on '{player}': {}", events.iter().filter(|event| matches!(event, LogEvent::Kill { killer_id: 1022, victim_name, .. } if victim_name == &player)).count());
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


    fn assert_mock_summaries(events: Vec<LogEvent>, expected_summaries: Vec<GameMatchSummary>) {
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
        events: Vec<LogEvent>,
    }
    impl TestDAL {
        /// Creates a new mock DAL for tests, yielding the all the `events`
        pub fn new(events: Vec<LogEvent>) -> Self {
            Self { events }
        }
    }
    impl Quake3ServerEvents for TestDAL {
        fn events_stream(self) -> Result<Pin<Box<dyn Stream<Item=Result<LogEvent>>>>> {
            let stream = stream::iter(self.events)
                .map(|event| Ok(event));
            Ok(Box::pin(stream))
        }
    }
}