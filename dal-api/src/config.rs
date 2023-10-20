//! Resting place for DAL's [Config] & friends

/// Configuration for the DAL crate
pub struct Config {

    /// The implementation to use when getting a Data Access Object (DAO) instance
    pub quake3_server_events_implementation: Quake3ServerEventsImplementations,

}

/// Here are some implementations -- real and imaginary examples of the flexibility this architecture brings
pub enum Quake3ServerEventsImplementations {
    SyncLogFileReader,
    ASyncLogFileReader,
    HttpRealtimeBinaryEventsReader,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            quake3_server_events_implementation: Quake3ServerEventsImplementations::SyncLogFileReader,
        }
    }
}