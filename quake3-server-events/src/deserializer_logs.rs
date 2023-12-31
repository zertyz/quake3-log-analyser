//! Common functions for parsing of Quake 3 Log files.
//!
//! See `benches/quake3_server_event_parsing.rs` for the study of trade-offs between Regex & `str::split*()`

use crate::types::Quake3FullEvents;
use std::{
    borrow::Cow,
    collections::BTreeMap,
    str::FromStr,
};


/// Transforms raw Quake 3 Log lines into the appropriate [model::quake3_logs::LogEvent] variants,
/// returning any errors that prevents the correct parsing.
pub fn deserialize_log_line<'a>(log_line: &str) -> Result<Quake3FullEvents<'a>, LogParsingError> {
    let log_line = log_line.trim_start_matches(" ");
    if log_line.len() == 0 {
        return Err(LogParsingError::EmptyLine)
    }

    let (_time, event_name_and_data) = log_line.split_once(" ")
        .map_or(Err(LogParsingError::UnrecognizedLineFormat), Ok)?;
    if event_name_and_data.starts_with("-") {
        return Ok(Quake3FullEvents::Comment)
    }
    let (event_name, data) = event_name_and_data.split_once(":")
        .map_or(Err(LogParsingError::UnrecognizedLineFormat), Ok)?;
    from_parts(event_name, data.trim_start_matches(" "))
        .map_err(|event_parsing_error| LogParsingError::EventParsingError { event_name: event_name.to_string(), event_parsing_error })
}

/// The errors that could prevent the parsing of a log line
#[derive(Debug, PartialEq)]
pub enum LogParsingError {
    EmptyLine,
    UnrecognizedLineFormat,
    MandatoryFieldIsEmpty { field_name: &'static str },
    UnparseableTime { field_name: &'static str, observed_number: String },
    EventParsingError { event_name: String, event_parsing_error: EventParsingError },
}

/// The errors that could preventing the parsing of an event in a log line
#[derive(Debug, PartialEq)]
pub enum EventParsingError {
    UnknownEventName,
    UnparseableNumber { key_name: &'static str, observed_data: String },
    AbsentKey { key_name: &'static str },
    UnknownDataFormat { description: String },
}

fn from_parts<'a>(event_name: &str, data: &str) -> Result<Quake3FullEvents<'a>, EventParsingError> {
    match event_name {
        "InitGame" => {
            let map = map_from_kv_data(data);
            Ok(Quake3FullEvents::InitGame {
                frag_limit: map.get("fraglimit").and_then(|n| number_from(n)),
                capture_limit: map.get("capturelimit").and_then(|n| number_from(n)),
                time_limit_min: map.get("timelimit").and_then(|n| number_from(n)),
            })
        },
        "ClientConnect" => {
            number_from(data)
                .map(|id| Quake3FullEvents::ClientConnect { id })
                .ok_or_else(|| EventParsingError::UnparseableNumber { key_name: "client id", observed_data: data.to_string() })
        },
        "ClientUserinfoChanged" => {
            let (numeric, textual) = data.split_once(" ")
                .ok_or_else(|| EventParsingError::UnknownDataFormat { description: format!("event data doesn't appear to be in the form <CLIENT_ID> <SPACE> key1\\val1\\key2\\val2\\...: log data: '{data}'")})?;
            let id = number_from(numeric)
                .ok_or_else(|| EventParsingError::UnparseableNumber { key_name: "client id", observed_data: numeric.to_string() })?;
            let map = map_from_kv_data(textual);
            map.get("n")
                .map(|name| Quake3FullEvents::ClientUserinfoChanged { id, name: Cow::Owned(name.to_string()) })
                .ok_or_else(|| EventParsingError::AbsentKey { key_name: "n" })
        },
        "ClientBegin" => {
            number_from(data)
                .map(|id| Quake3FullEvents::ClientBegin { id })
                .ok_or_else(|| EventParsingError::UnparseableNumber { key_name: "client id", observed_data: data.to_string() })
        }
        "ClientDisconnect" => {
            number_from(data)
                .map(|id| Quake3FullEvents::ClientDisconnect { id })
                .ok_or_else(|| EventParsingError::UnparseableNumber { key_name: "client id", observed_data: data.to_string() })
        },
        "Item" => Ok(Quake3FullEvents::Item),
        "say" => Ok(Quake3FullEvents::Say),
        "Kill" => {
            let (
                    killer_id,
                    victim_id,
                    reason_id,
                    text_description
            ) = {
                let data_format_error = || EventParsingError::UnknownDataFormat { description: format!("`Kill` data doesn't appear to be in the form '<KILLER_ID> <VICTIM_ID> <REASON_ID>: <TEXT_DESCRIPTION>': data is '{data}'") };
                let parsing_error_generator = |field_name| move |parsing_err| Err(EventParsingError::UnknownDataFormat { description: format!("Can't parse {field_name} from `Kill` data in the form '<KILLER_ID> <VICTIM_ID> <REASON_ID>: <TEXT_DESCRIPTION>' -- '{data}': {parsing_err}") });
                let mut parts = data.splitn(4, " ");
                (
                    parts.next().ok_or_else(data_format_error)?
                        .parse::<u32>().or_else(parsing_error_generator("KILLER_ID"))?,
                    parts.next().ok_or_else(data_format_error)?
                        .parse::<u32>().or_else(parsing_error_generator("VICTIM_ID"))?,
                    parts.next().ok_or_else(data_format_error)?
                        .strip_suffix(":").ok_or_else(data_format_error)?
                        .parse::<u32>().or_else(parsing_error_generator("REASON_ID"))?,
                    parts.next().ok_or_else(data_format_error)?
                )
            };
            let (killer_name, victim_name, reason_name) = {
                let text_description_format_error = || EventParsingError::UnknownDataFormat { description: format!("Text description in `Kill` data appears not to be in the form '<KILLER_NAME> killed <VICTIM_NAME> by <REASON_NAME>' -- it was '{text_description}'") };
                let (killer_name, reminder) = text_description.split_once(" killed ")
                    .ok_or_else(text_description_format_error)?;
                let (victim_name, reason_name) = reminder.rsplit_once(" by ")
                    .ok_or_else(text_description_format_error)?;
                (killer_name.to_string(), victim_name.to_string(), reason_name.to_string())
            };
            Ok(Quake3FullEvents::Kill {
                killer_id,
                victim_id,
                reason_id,
                killer_name: Cow::Owned(killer_name),
                victim_name: Cow::Owned(victim_name),
                reason_name: Cow::Owned(reason_name),
            })
        },
        "Exit" => Ok(Quake3FullEvents::Exit),
        "red" => {
            let (red_value, blue_key_value) = data.split_once(" ")
                .ok_or_else(|| EventParsingError::UnknownDataFormat { description: format!("event doesn't appear to be in the form 'red:n blue:n': log line: 'red:{data}'")})?;
            let red = number_from(red_value)
                .ok_or_else(|| EventParsingError::UnparseableNumber { key_name: "red score", observed_data: red_value.to_string() })?;
            let blue_value = blue_key_value.split(":").skip(1).next()
                .ok_or_else(|| EventParsingError::UnknownDataFormat { description: format!("data couldn't be split into key and value for the blue score -- '{blue_key_value}'") })?;
            let blue= number_from(blue_value)
                .ok_or_else(|| EventParsingError::UnparseableNumber { key_name: "blue score", observed_data: blue_value.to_string() })?;
            Ok(Quake3FullEvents::CaptureTheFlagResults { red, blue })
        },
        "score" => {
            let (frags_value, data) = data.split_once(" ")
                .ok_or_else(|| EventParsingError::UnknownDataFormat { description: format!("event doesn't appear to be in the form 'score: n  ping: n  client: n name': log line: 'score:{data}'")})?;
            let frags = number_from(frags_value)
                .ok_or_else(|| EventParsingError::UnparseableNumber { key_name: "frags", observed_data: frags_value.to_string() })?;
            let client_values = data.split(": ").skip(2).next()
                .ok_or_else(|| EventParsingError::UnknownDataFormat { description: format!("couldn't extract client values out of `data` -- '{data}'") })?;
            let (client_id_value, client_name) = client_values.split_once(" ")
                .ok_or_else(|| EventParsingError::UnknownDataFormat { description: format!("couldn't split client id and name out of `client_values` -- '{client_values}'") })?;
            let client_id = number_from(client_id_value)
                .ok_or_else(|| EventParsingError::UnparseableNumber { key_name: "client_id", observed_data: client_id_value.to_string() })?;
            Ok(Quake3FullEvents::Score {frags, id: client_id, name: Cow::Owned(client_name.to_owned())} )
        },
        "ShutdownGame" => Ok(Quake3FullEvents::ShutdownGame),
        _ => Err(EventParsingError::UnknownEventName),
    }
}


fn map_from_kv_data(data: &str) -> BTreeMap<&str, &str> {
    let iter = data.split("\\");
    let kv_iter = iter.clone().zip(iter.skip(1));
    BTreeMap::from_iter(kv_iter.map(|(k, v)| (k, v)))
}

fn number_from<T: FromStr>(number: &str) -> Option<T> {
    number.parse()
        .map_or_else(
            |_err| None,
            |n| Some(n)
        )
}

/// Unit tests for the [deserializer](super) module
#[cfg(test)]
mod tests {
    use super::*;


    // valid messages use cases
    ///////////////////////////
    // the tests bellow checks valid log messages, to assure the implementation
    // is able to parse the events correctly


    /// Tests that the time parser is able to handle hours without the padding zero and even with 3 digits
    #[test]
    fn unconventional_hours() {
        assert_log_parsing(r#"  0:37 ------------------------------------------------------------"#, Quake3FullEvents::Comment);
        assert_log_parsing(r#" 80:37 ------------------------------------------------------------"#, Quake3FullEvents::Comment);
        assert_log_parsing(r#"980:37 ------------------------------------------------------------"#, Quake3FullEvents::Comment);
    }

    /// Tests that comment messages are correctly identified
    #[test]
    fn comment() {
        assert_log_parsing(r#"20:37 ------------------------------------------------------------"#, Quake3FullEvents::Comment);
    }

    /// Tests the [Quake3Events::InitGame] messages for each of the game types: Death match vs Capture the flag.
    #[test]
    fn init_game() {
        // death match
        assert_log_parsing(r#" 1:47 InitGame: \sv_floodProtect\1\sv_maxPing\0\sv_minPing\0\sv_maxRate\10000\sv_minRate\0\sv_hostname\Code Miner Server\g_gametype\0\sv_privateClients\2\sv_maxclients\16\sv_allowDownload\0\bot_minplayers\0\dmflags\0\fraglimit\20\timelimit\15\g_maxGameClients\0\capturelimit\8\version\ioq3 1.36 linux-x86_64 Apr 12 2009\protocol\68\mapname\q3dm17\gamename\baseq3\g_needpass\0"#,
                           Quake3FullEvents::InitGame {
                               frag_limit: Some(20),
                               capture_limit: Some(8),
                               time_limit_min: Some(15),
                           });
        // capture the flag
        assert_log_parsing(r#" 2:33 InitGame: \capturelimit\8\g_maxGameClients\0\timelimit\15\fraglimit\20\dmflags\0\bot_minplayers\0\sv_allowDownload\0\sv_maxclients\16\sv_privateClients\2\g_gametype\4\sv_hostname\Code Miner Server\sv_minRate\0\sv_maxRate\10000\sv_minPing\0\sv_maxPing\0\sv_floodProtect\1\version\ioq3 1.36 linux-x86_64 Apr 12 2009\protocol\68\mapname\Q3TOURNEY6_CTF\gamename\baseq3\g_needpass\0"#,
                           Quake3FullEvents::InitGame {
                               frag_limit: Some(20),
                               capture_limit: Some(8),
                               time_limit_min: Some(15),
                           });
    }
    
    #[test]
    fn client_connect() {
        assert_log_parsing(r#" 2:33 ClientConnect: 2"#, Quake3FullEvents::ClientConnect {id: 2});
    }

    #[test]
    fn client_info() {
        assert_log_parsing(r#"2:33 ClientUserinfoChanged: 2 n\Isgalamido\t\1\model\uriel/zael\hmodel\uriel/zael\g_redteam\\g_blueteam\\c1\5\c2\5\hc\100\w\0\l\0\tt\0\tl\0"#,
                           Quake3FullEvents::ClientUserinfoChanged { id: 2, name: "Isgalamido".into() })
    }

    #[test]
    fn client_begin() {
        assert_log_parsing(r#" 2:33 ClientBegin: 2"#, Quake3FullEvents::ClientBegin {id: 2})
    }

    #[test]
    fn client_disconnect() {
        assert_log_parsing(r#" 2:33 ClientDisconnect: 2"#, Quake3FullEvents::ClientDisconnect {id: 2});
    }

    #[test]
    fn item() {
        assert_log_parsing(r#" 2:36 Item: 2 ammo_rockets"#, Quake3FullEvents::Item)
    }

    #[test]
    fn say() {
        assert_log_parsing(r#"981:26 say: Isgalamido: team blue"#, Quake3FullEvents::Say)
    }

    #[test]
    fn kill_event() {
        assert_log_parsing(r#"20:54 Kill: 1022 2 22: <world> killed Isgalamido by MOD_TRIGGER_HURT"#,
                           Quake3FullEvents::Kill {
                               killer_id: 1022,
                               victim_id: 2,
                               reason_id: 22,
                               killer_name: "<world>".into(),
                               victim_name: "Isgalamido".into(),
                               reason_name: "MOD_TRIGGER_HURT".into(),
                           });
    }

    #[test]
    fn exit() {
        assert_log_parsing(r#"10:12 Exit: Capturelimit hit."#, Quake3FullEvents::Exit)
    }
    
    #[test]
    fn capture_the_flag_score() {
        assert_log_parsing(r#"10:12 red:8  blue:6"#, Quake3FullEvents::CaptureTheFlagResults { red: 8, blue: 6 })
    }

    /// Test scores with either positive or negative frags
    #[test]
    fn score() {
        assert_log_parsing(r#"10:12 score: 77  ping: 3  client: 2 Isgalamido"#, Quake3FullEvents::Score { frags: 77, id: 2, name: "Isgalamido".into() });
        assert_log_parsing(r#"10:12 score: -77  ping: 3  client: 5 Dono da Bola"#, Quake3FullEvents::Score { frags: -77, id: 5, name: "Dono da Bola".into() })
    }

    #[test]
    fn shutdown() {
        assert_log_parsing(r#"10:28 ShutdownGame:"#, Quake3FullEvents::ShutdownGame)
    }


    fn assert_log_parsing(log_line: &str, expected_log_event: Quake3FullEvents) {
        let deserialization_result = deserialize_log_line(log_line);
        assert!(deserialization_result.is_ok(), "Log line '{log_line}' couldn't be deserialized: LogParsingError::{:?}", deserialization_result.unwrap_err());
        assert_eq!(deserialization_result.unwrap(), expected_log_event, "Log line '{log_line}' wasn't correctly deserialized");
    }


    // malformed messages use cases
    ///////////////////////////////
    // the tests bellow present invalid log lines, to assure the implementation
    // won't break and is able to present meaningful error messages

    /// Tests that empty lines are correctly detected & handled
    #[test]
    fn empty_line() {
        assert_log_parsing_error(r#""#, LogParsingError::EmptyLine);
    }

    /// Tests that log lines out of the usual pattern are correctly identified & handled
    #[test]
    fn misformatted() {
        assert_log_parsing_error(r#"20:37------------------------------------------------------------"#, LogParsingError::UnrecognizedLineFormat);
        assert_log_parsing_error(r#"------------------------------------------------------------"#, LogParsingError::UnrecognizedLineFormat);
        assert_log_parsing_error(r#"any: info"#, LogParsingError::UnrecognizedLineFormat);
    }

    /// Tests that unknown events in the log data are correctly identified
    #[test]
    fn unknown_event() {
        // death match
        assert_log_parsing_error(r#" 1:47 Init_Game: \sv_floodProtect\1\sv_maxPing\0\sv_minPing\0\sv_maxRate\10000\sv_minRate\0\sv_hostname\Code Miner Server\g_gametype\0\sv_privateClients\2\sv_maxclients\16\sv_allowDownload\0\bot_minplayers\0\dmflags\0\fraglimit\20\timelimit\15\g_maxGameClients\0\capturelimit\8\version\ioq3 1.36 linux-x86_64 Apr 12 2009\protocol\68\mapname\q3dm17\gamename\baseq3\g_needpass\0"#,
                                 LogParsingError::EventParsingError { event_name: "Init_Game".to_string(), event_parsing_error: EventParsingError::UnknownEventName });
    }

    /// Tests the [Quake3Events::InitGame] messages with unparseable data are correctly identified, reported and handled
    #[test]
    fn bad_client_connect() {
        // text in id
        assert_log_parsing_error(r#" 2:33 ClientConnect: 2a"#,
                                 LogParsingError::EventParsingError { event_name: String::from("ClientConnect"), event_parsing_error: EventParsingError::UnparseableNumber { key_name: "client id", observed_data: String::from("2a") } });
        // extra space
        assert_log_parsing_error(r#" 2:33 ClientConnect: _2"#,
                                 LogParsingError::EventParsingError { event_name: String::from("ClientConnect"), event_parsing_error: EventParsingError::UnparseableNumber { key_name: "client id", observed_data: String::from("_2") } });
    }

    #[test]
    fn bad_client_info() {
        // no name -- no `n` key
        assert_log_parsing_error(r#"2:33 ClientUserinfoChanged: 2 not_n\Isgalamido\t\1\model\uriel/zael\hmodel\uriel/zael\g_redteam\\g_blueteam\\c1\5\c2\5\hc\100\w\0\l\0\tt\0\tl\0"#,
                                 LogParsingError::EventParsingError {
                                     event_name: String::from("ClientUserinfoChanged"),
                                     event_parsing_error: EventParsingError::AbsentKey { key_name: "n" } });
        // no client id
        assert_log_parsing_error(r#"2:33 ClientUserinfoChanged: n\Isgalamido\t\1\model\uriel/zael\hmodel\uriel/zael\g_redteam\\g_blueteam\\c1\5\c2\5\hc\100\w\0\l\0\tt\0\tl\0"#,
                                 LogParsingError::EventParsingError {
                                     event_name: String::from("ClientUserinfoChanged"),
                                     event_parsing_error: EventParsingError::UnknownDataFormat {
                                         description: String::from(r#"event data doesn't appear to be in the form <CLIENT_ID> <SPACE> key1\val1\key2\val2\...: log data: 'n\Isgalamido\t\1\model\uriel/zael\hmodel\uriel/zael\g_redteam\\g_blueteam\\c1\5\c2\5\hc\100\w\0\l\0\tt\0\tl\0'"#)
                                     }
                                 });

        // unparseable client id
        assert_log_parsing_error(r#"2:33 ClientUserinfoChanged: _2_ n\Isgalamido\t\1\model\uriel/zael\hmodel\uriel/zael\g_redteam\\g_blueteam\\c1\5\c2\5\hc\100\w\0\l\0\tt\0\tl\0"#,
                                 LogParsingError::EventParsingError {
                                     event_name: String::from("ClientUserinfoChanged"),
                                     event_parsing_error: EventParsingError::UnparseableNumber {
                                         key_name: "client id",
                                         observed_data: "_2_".to_string()
                                     }
                                 });
    }



    fn assert_log_parsing_error(log_line: &str, expected_log_parsing_error: LogParsingError) {
        let deserialization_result = deserialize_log_line(log_line);
        assert!(deserialization_result.is_err(), "The bad log line '{log_line}' did not fail in the deserialization (as it should). The unexpected Ok parsing result was {:?}", deserialization_result.unwrap());
        assert_eq!(deserialization_result.unwrap_err(), expected_log_parsing_error, "The bad log line '{log_line}' did not produce the expected error");
    }

}