//! Factory for obtaining one of the implementations of our DAO objects

use dal_api::{Config, Quake3ServerEvents, Quake3ServerEventsImplementations};

/// Instantiates a Data Access Object (dao) able work on the contents of `log_locator`,
/// pointing to a Quake3 server log file, from which a [Quake3ServerEvents] is returned.
pub fn instantiate_log_dao(config: &Config, log_locator: &str) -> impl Quake3ServerEvents {
    match config.quake3_server_events_implementation {
        Quake3ServerEventsImplementations::SyncLogFileReader => crate::sync_file_reader::Quake3LogFileSyncReader::new(log_locator),
        Quake3ServerEventsImplementations::ASyncLogFileReader => todo!("Not implemented for this exercise"),
        Quake3ServerEventsImplementations::HttpRealtimeBinaryEventsReader => todo!("Not implemented for this exercise"),
    }
}