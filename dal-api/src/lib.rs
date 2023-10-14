use futures::Stream;
use model::quake3_logs::LogEvent;


/// Feed for Quake 3 server events
pub trait Quake3ServerEvents {

    /// Returns a `Stream` which yields Quake 3 server events
    fn events_stream() -> Box<dyn Stream<Item=LogEvent>>;
}