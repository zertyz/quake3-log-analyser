use std::fmt::Debug;
use std::pin::Pin;
use std::result;
use futures::Stream;
use model::quake3_logs::LogEvent;

pub type Result<T> = result::Result<T, Box<dyn std::error::Error>>;


/// Feed for Quake 3 server events
pub trait Quake3ServerEvents {


    /// Consumes this object, returning a `Stream` which yields Quake 3 server events
    fn events_stream(self) -> Result<Box<dyn Stream<Item=Result<LogEvent>>>>;
}
