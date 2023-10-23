//! Factory for obtaining one of the implementations of our DAO objects

use dal_api::{
    Config,
    Quake3ServerEvents,
    Quake3ServerEventsImplementations,
};
use std::sync::Arc;


/// Instantiates a Data Access Object (dao) able retrieve data from the given `implementation` source
pub fn instantiate_log_dao(implemetation: Quake3ServerEventsImplementations<'static>, config: Arc<Config>) -> Box<dyn Quake3ServerEvents + 'static> {
    match implemetation {
        Quake3ServerEventsImplementations::StdinReader => crate::stdin_reader::Quake3LogFileStdinReader::new(config),
        Quake3ServerEventsImplementations::SyncLogFileReader(params) => crate::sync_file_reader::Quake3LogFileSyncReader::new(config, params),
        Quake3ServerEventsImplementations::AsyncLogFileReader(_params) => todo!("Not implemented for this exercise"),
        Quake3ServerEventsImplementations::HttpRealtimeBinaryEventsReader => todo!("Not implemented for this exercise"),
    }
}