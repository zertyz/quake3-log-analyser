use model::types::Result;
use dal_api::Quake3ServerEvents;
use quake3_server_log::{
    model::LogEvent,
    deserializer::{deserialize_log_line, LogParsingError},
};
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::pin::Pin;
use std::task::Poll;
use futures::{FutureExt, Stream, stream, StreamExt};


/// Size for buffering IO (the larger, more RAM is used, but fewer system calls / context switches / hardware requests are required)
const BUFFER_SIZE: usize = 1024*1024;


pub struct Quake3LogFileSyncReader {
    log_file_path: String,
}

impl Quake3LogFileSyncReader {

    pub fn new<IntoString: Into<String>>(log_file_path: IntoString) -> Self {
        Self {
            log_file_path: log_file_path.into()
        }
    }

}

impl Quake3ServerEvents for Quake3LogFileSyncReader {

    fn events_stream(self) -> Result<Pin<Box<dyn Stream<Item=Result<LogEvent>>>>> {
        let file = File::open(&self.log_file_path)
            .map_err(|err| format!("Couldn't open Quake3 Server log file '{}' for reading: {err}", self.log_file_path))?;
        let reader = BufReader::with_capacity(BUFFER_SIZE, file);
        let mut lines_iter = reader.lines().enumerate();

        let yield_item = |event| Poll::Ready(Some(Ok(event)));
        let yield_error = |err| Poll::Ready(Some(Err(Box::from(err))));
        let end_of_stream = || Poll::Ready(None);

        let stream = stream::poll_fn(move |_|
            lines_iter.next()
                .map_or_else(end_of_stream,
                             |(line_number, line_result)| line_result
                                 .map_err(|read_err| format!("IO read error when processing log file '{}' at line {}: {read_err:?}", self.log_file_path, line_number+1))
                                 .map_or_else(yield_error,
                                              |line| deserialize_log_line(&line)
                                                     .map_err(|log_parser_err| format!("`LogParsingError` when processing log file '{}' at line {}: {log_parser_err:?}", self.log_file_path, line_number+1))
                                                     .map_or_else(yield_error, yield_item)

                                 )
                )
        )
/*            .inspect(|event| eprintln!("{event:?}"))*/;
        Ok(Box::pin(stream))
    }

}


/// Unit tests the [sync_file_reader](super) implementation of [dal_api::Quake3ServerEvents]
#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use super::*;


    /// The location of a good log file, with all lines OK
    const GOOD_LOG_FILE_LOCATION: &str = "tests/resources/qgames_excerpt.log";
    const MALFORMED_LOG_FILE_LOCATION: &str = "tests/resources/malformed_line.log";
    const NON_EXISTING_FILE_LOCATION: &str = "/tmp/non-existing.log";


    /// Tests that an existing & valid file (for which there will be no IO errors) may be correctly read from beginning to end
    #[test]
    fn read_file() {
        let log_dao = Quake3LogFileSyncReader::new(GOOD_LOG_FILE_LOCATION);
        let stream = log_dao.events_stream().expect("Couldn't create the `Stream`");
        let stream = futures::executor::block_on_stream(Pin::from(stream));
        let log_lines_count = stream.enumerate()
            .inspect(|(line_number, event_result)| assert!(event_result.is_ok(), "Parsing log line #{} yielded a unexpected result {event_result:?}", line_number+1))
            .count();
        assert_eq!(log_lines_count, 100, "Unexpected number of parsed log lines");
    }

    /// Tests that opening a non-existing file yields the expected error result & message
    #[test]
    fn non_existing_file() {
        let expected_err = "Couldn't open Quake3 Server log file '/tmp/non-existing.log' for reading: No such file or directory (os error 2)";
        let log_dao = Quake3LogFileSyncReader::new(NON_EXISTING_FILE_LOCATION);
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
        let log_dao = Quake3LogFileSyncReader::new(MALFORMED_LOG_FILE_LOCATION);
        let stream = log_dao.events_stream().expect("Couldn't create the `Stream`");
        let stream = futures::executor::block_on_stream(Pin::from(stream));
        let log_lines_count = stream.enumerate()
            .inspect(|(line_number, event_result)| {
                if let Some(expected_error) = expected_lines_and_errors.remove(&(line_number+1)) {
                    assert!(event_result.is_err(), "Parsing the malformed log line #{} went unreported -- the parser said all was good: {event_result:?}", line_number+1);
                    assert_eq!(event_result.as_ref().unwrap_err().to_string(), expected_error.to_string(), "Error report differs at the malformed line #{}", line_number+1)
                } else {
                    assert!(event_result.is_ok(), "Parsing log line #{} yielded a unexpected result {event_result:?}", line_number+1)
                }
            })
            .count();
        assert_eq!(log_lines_count, 7, "Unexpected number of parsed log lines");
        assert!(expected_lines_and_errors.len() == 0, "Not all expected errors were cought: {} are left: {:?}", expected_lines_and_errors.len(), expected_lines_and_errors);
    }

}