//! Resting place for [Quake3LogSyncReader]


use crate::events_translation::translate_quake3_events;
use common::types::Result;
use model::quake3_events::Quake3Events;
use dal_api::{Config, Quake3ServerEvents};
use quake3_server_log::deserializer_logs::deserialize_log_line;
use std::{
    pin::Pin,
    sync::Arc,
    task::Poll,
};
use futures::{Stream, stream, StreamExt};
use log::trace;


/// [Quake3ServerEvents] implementation for reading Quake 3 Server events from a log file
pub struct Quake3LogSyncReader<Reader: std::io::BufRead> {
    config: Arc<Config>,
    source_name: String,
    reader: Reader,
}

impl<Reader: std::io::BufRead> Quake3LogSyncReader<Reader> {

    pub fn new(config: Arc<Config>, source_name: &str, reader: Reader) -> Box<Self> {
        Box::new(Self {
            config,
            source_name: source_name.into(),
            reader,
        })
    }

}

impl<Reader: std::io::BufRead + 'static> Quake3ServerEvents for Quake3LogSyncReader<Reader> {

    fn events_stream(self: Box<Self>) -> Result<Pin<Box<dyn Stream<Item=Quake3Events<'static>>>>> {
        let mut lines_iter = self.reader.lines().enumerate();

        let yield_item = |event| Poll::Ready(Some(Ok(event)));
        let yield_error = |err| Poll::Ready(Some(Err(Box::from(err))));
        let end_of_stream = || Poll::Ready(None);

        let debug = self.config.debug;
        let source_name = self.source_name.to_owned();
        let stream = stream::poll_fn(move |_|
            lines_iter.next()
                .map_or_else(end_of_stream,
                             |(line_number, line_result)| line_result
                                 .map_err(|read_err| format!("IO read error when processing log file '{}' at line {}: {read_err:?}", source_name, line_number+1))
                                 .map_or_else(yield_error,
                                              |line| deserialize_log_line(&line)
                                                     .map_err(|log_parser_err| format!("`LogParsingError` when processing log file '{}' at line {}: {log_parser_err:?}", source_name, line_number+1))
                                                     .map_or_else(yield_error, yield_item)

                                 )
                )
        );
        let stream = translate_quake3_events(stream);
        let stream: Pin<Box<dyn Stream<Item=Quake3Events<'static>>>> = if debug {
            Box::pin(stream
                .inspect(|yielded_event| trace!("{yielded_event:?}")))
        } else {
            Box::pin(stream)
        };
        Ok(stream)
    }

}

// for unit tests, see sync_file_reader.rs
// (the tests were delegated there as it is easier to test from files)