//! Resting place for [Quake3LogFileStdinReader]


use crate::sync_reader::Quake3LogSyncReader;
use common::types::Result;
use model::{
    quake3_events::Quake3Events,
};
use dal_api::{Config, Quake3ServerEvents};
use std::io::BufReader;
use std::pin::Pin;
use std::sync::Arc;
use futures::Stream;

/// Size for buffering IO (the larger, more RAM is used, but fewer system calls / context switches / hardware requests are required)
const BUFFER_SIZE: usize = 1024*1024;

/// [Quake3ServerEvents] implementation for reading Quake 3 Server events from a log file
pub struct Quake3LogFileStdinReader {
    config: Arc<Config>,
}

impl Quake3LogFileStdinReader {

    pub fn new(config: Arc<Config>) -> Box<Self> {
        Box::new(Self {
            config,
        })
    }

}

impl Quake3ServerEvents for Quake3LogFileStdinReader {

    fn events_stream(self: Box<Self>) -> Result<Pin<Box<dyn Stream<Item=Quake3Events<'static>>>>> {
        let reader = BufReader::with_capacity(BUFFER_SIZE, std::io::stdin());
        Quake3LogSyncReader::new(self.config, "<stdin>", reader)
            .events_stream()
    }

}