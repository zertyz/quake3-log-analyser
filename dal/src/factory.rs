//! Factory for obtaining one of the implementations of our DAO objects

use std::pin::Pin;
use std::sync::Arc;
use dal_api::{Config, Quake3ServerEvents, Quake3ServerEventsImplementations};

/// Instantiates a Data Access Object (dao) able work on the contents of `log_locator`,
/// pointing to a Quake3 server log file, from which a [Quake3ServerEvents] is returned.
pub fn instantiate_log_dao(implemetation: Quake3ServerEventsImplementations<'static>, config: Arc<Config>) -> Pin<Box<dyn Quake3ServerEvents + 'static>> {
    match implemetation {
        Quake3ServerEventsImplementations::StdinReader => todo!("DO IT"),
        Quake3ServerEventsImplementations::SyncLogFileReader(params) => crate::sync_file_reader::Quake3LogFileSyncReader::new(config, params),
        Quake3ServerEventsImplementations::AsyncLogFileReader(params) => todo!("Not implemented for this exercise"),
        Quake3ServerEventsImplementations::HttpRealtimeBinaryEventsReader => todo!("Not implemented for this exercise"),
    }
}