//! Resting place for [Quake3LogFileSyncReader]


use common::types::Result;
use model::{
    quake3_events::Quake3Events,
};
use dal_api::{Config, FileReaderInfo, Quake3ServerEvents};
use quake3_server_log::{
    types::Quake3FullEvents,
    deserializer::{deserialize_log_line, LogParsingError},
};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::pin::Pin;
use std::sync::Arc;
use std::task::Poll;
use futures::{FutureExt, Stream, stream, StreamExt};
use log::trace;
use crate::events_translation::translate_quake3_events;
use crate::sync_reader::Quake3LogSyncReader;


/// Size for buffering IO (the larger, more RAM is used, but fewer system calls / context switches / hardware requests are required)
const BUFFER_SIZE: usize = 1024*1024;


/// [Quake3ServerEvents] implementation for reading Quake 3 Server events from a log file
pub struct Quake3LogFileSyncReader<'a> {
    config: Arc<Config>,
    params: FileReaderInfo<'a>,
}

impl<'a> Quake3LogFileSyncReader<'a> {

    pub fn new(config: Arc<Config>, params: FileReaderInfo<'a>) -> Box<Self> {
        Box::new(Self {
            config,
            params,
        })
    }

}

impl Quake3ServerEvents for Quake3LogFileSyncReader<'static> {

    fn events_stream(self: Box<Self>) -> Result<Pin<Box<dyn Stream<Item=Quake3Events<'static>>>>> {
        let file = File::open(&self.params.log_file_path.as_ref())
            .map_err(|err| format!("Couldn't open Quake3 Server log file '{}' for reading: {err}", self.params.log_file_path))?;
        let reader = BufReader::with_capacity(BUFFER_SIZE, file);
        Quake3LogSyncReader::new(self.config, &self.params.log_file_path, reader)
            .events_stream()
    }

}


/// Unit tests the [sync_file_reader](super) implementation of [dal_api::Quake3ServerEvents]
#[cfg(test)]
mod tests {
    use std::borrow::Cow;
    use std::collections::HashMap;
    use super::*;


    /// The location of a good log file, with all lines OK
    const GOOD_LOG_FILE_LOCATION: &str = "tests/resources/qgames_excerpt.log";
    const MALFORMED_LOG_FILE_LOCATION: &str = "tests/resources/malformed_line.log";
    const NON_EXISTING_FILE_LOCATION: &str = "/tmp/non-existing.log";


    /// Tests that an existing & valid file (for which there will be no IO errors) may be correctly read from beginning to end
    #[test]
    fn read_file() {
        let log_dao = Quake3LogFileSyncReader::new(config(), FileReaderInfo { log_file_path: Cow::Borrowed(GOOD_LOG_FILE_LOCATION) });
        let stream = log_dao.events_stream().expect("Couldn't create the `Stream`");
        let stream = futures::executor::block_on_stream(Pin::from(stream));
        let events_count = stream
            .inspect(|event| assert!(event.is_ok(), "Parsing log line #{} yielded a unexpected error {event:?}", event.event_id()))
            .count();
        assert_eq!(events_count, 32, "Unexpected number of parsed log lines");
    }

    /// Tests that opening a non-existing file yields the expected error result & message
    #[test]
    fn non_existing_file() {
        let expected_err = "Couldn't open Quake3 Server log file '/tmp/non-existing.log' for reading: No such file or directory (os error 2)";
        let log_dao = Quake3LogFileSyncReader::new(config(), FileReaderInfo { log_file_path: Cow::Borrowed(NON_EXISTING_FILE_LOCATION) });
        match log_dao.events_stream() {
            Ok(stream) => panic!("Opening a non-existing file was expected to fail at `Stream` creation, but the operation succeeded"),
            Err(stream_creation_err) => assert_eq!(stream_creation_err.to_string(), expected_err.to_string(), "Unexpected `Stream` creation error"),
        }
    }

    /// Tests that errors in the parser (due to log file contents) are exposed to the caller and allows the `Stream` to continue
    #[test]
    fn malformed_lines() {
        let mut expected_lines_and_errors = HashMap::from([
            (2, r#"`LogParsingError` when processing log file 'tests/resources/malformed_line.log' at line 2: EventParsingError { event_name: " 0", event_parsing_error: UnknownEventName }"#),
            (5, r#"`LogParsingError` when processing log file 'tests/resources/malformed_line.log' at line 5: EventParsingError { event_name: "ClientUserinfoChanged", event_parsing_error: UnparseableNumber { key_name: "client id", observed_data: "3_" } }"#),
            (6, r#"`LogParsingError` when processing log file 'tests/resources/malformed_line.log' at line 6: EventParsingError { event_name: "ClientUserinfoChanged", event_parsing_error: UnknownDataFormat { description: "event data doesn't appear to be in the form <CLIENT_ID> <SPACE> key1\\val1\\key2\\val2\\...: log data: 'n\\Mocinha\\t\\0\\model\\sarge\\hmodel\\sarge\\g_redteam\\\\g_blueteam\\\\c1\\4\\c2\\5\\hc\\95\\w\\0\\l\\0\\tt\\0\\tl\\0'" } }"#)
        ]);
        let log_dao = Quake3LogFileSyncReader::new(config(), FileReaderInfo { log_file_path: Cow::Borrowed(MALFORMED_LOG_FILE_LOCATION) });
        let stream = log_dao.events_stream().expect("Couldn't create the `Stream`");
        let stream = futures::executor::block_on_stream(Pin::from(stream));
        let events_count = stream
            .inspect(|event| {
                let line_number = event.event_id();
                if let Some(expected_error) = expected_lines_and_errors.remove(&line_number) {
                    assert!(event.is_err(), "Parsing the malformed log line #{line_number} went unreported -- the parser said all was good: {event:?}");
                    assert_eq!(event.unwrap_err().to_string(), expected_error.to_string(), "Error report differs at the malformed line #{line_number}")
                } else {
                    assert!(event.is_ok(), "Parsing log line #{line_number} yielded a unexpected result {event:?}")
                }
            })
            .count();
        assert_eq!(events_count, 5, "Unexpected number of events");
        assert!(expected_lines_and_errors.len() == 0, "Not all expected errors were cought: {} are left: {:?}", expected_lines_and_errors.len(), expected_lines_and_errors);
    }

    
    fn config() -> Arc<Config> {
        Arc::new(Config {
            debug: false,
        })
    }
    
}