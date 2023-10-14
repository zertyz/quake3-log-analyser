use std::collections::BTreeMap;
use std::str::FromStr;
use futures::{Stream, stream, TryFutureExt};
use dal_api::Quake3ServerEvents;
use model::quake3_logs::LogEvent;

struct Quake3ServerEventsLogFileReader {

}

impl Quake3ServerEvents for Quake3ServerEventsLogFileReader {

    fn events_stream() -> Box<dyn Stream<Item=LogEvent>> {
        Box::new(stream::iter([]))
    }

}

struct LogEventParser {}
impl LogEventParser {

    /// Tells which data format is used for each event
    pub fn from_parts(event_name: &str, data: &[&str]) -> Option<LogEvent> {
        match event_name {
            "InitGame" => {
                let map = Self::map_from(data);
                Some(LogEvent::InitGame {
                    frag_limit: map.get("fraglimit").and_then(|n| Self::number_from(n)),
                    capture_limit: map.get("capturelimit").and_then(|n| Self::number_from(n)),
                    time_limit_min: map.get("timelimit").and_then(|n| Self::number_from(n)),
                })
            },
            "ClientConnect" => {
                Self::number_from(data[0])
                    .map(|id| LogEvent::ClientConnect { id })
                // .err: could not parse id from the ClientConnect message
            },
            "ClientUserinfoChanged" => {
                let map = Self::map_from(data);
                map.get("n")
                    .map(|name| LogEvent::ClientUserinfoChanged { name: name.to_string() })
                // .err: Name `n` is not present in a ClientUserinfoChanged message
            },
            "ClientDisconnect" => {
                Self::number_from(data[0])
                    .map(|id| LogEvent::ClientConnect { id })
                // .err: could not parse id from the ClientDisconnect message
            },
            "Kill" => Some(LogEvent::Kill),
            "Exit" => Some(LogEvent::Exit),
            "score" => Some(LogEvent::Score),
            "ShutdownGame" => Some(LogEvent::ShutdownGame),
            _ => None,
        }
    }

    fn map_from(data: &[&str]) -> BTreeMap<String, String> {
        BTreeMap::new()
    }

    fn number_from<T: FromStr>
                  (number: &str) -> Option<T> {
        number.parse()
            .map_or_else(
                |err| None,
                |n| Some(n)
            )
    }

}
