//! Resting place for DAL's [Config] & friends

use std::borrow::Cow;

/// Configuration for the DAL crate
pub struct Config {

    /// Outputs the data given out to users
    pub debug: bool,

}

/// Here are some implementations -- real and imaginary examples of the flexibility this architecture brings
pub enum Quake3ServerEventsImplementations<'a> {
    /// Reads events (as present in a log file) from stdin
    StdinReader,
    /// Reads events from Quake 3 server log files, using the sync / std APIs
    SyncLogFileReader(FileReaderInfo<'a>),
    /// Reads events from Quake 3 server log files, using the async / tokio APIs
    AsyncLogFileReader(FileReaderInfo<'a>),
    // /// Reads events (as presented in a log file) using the sync / std buffered reader
    // SyncReader { reader: Box<dyn std::io::BufRead> },
    // /// Reads events (as presented in a log file) using the sync / tokio buffered reader
    // AsyncReader { reader: Box<dyn std::io::BufRead> },
    /// Reads Quake 3 server events from an undergoing game (hypothetical, just to demonstrate the flexibility of the Factory Pattern)
    HttpRealtimeBinaryEventsReader,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            debug: false,
        }
    }
}

/// Information for instantiating DAL implementations that reads files
pub struct FileReaderInfo<'a> {
    pub log_file_path: Cow<'a, str>,
}

// /// Information for instantiating DAL implementations that reads from buffered Readers