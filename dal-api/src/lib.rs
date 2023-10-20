use model::types::Result;
use std::fmt::Debug;
use std::pin::Pin;
use futures::Stream;
use quake3_server_log::model::Quake3Events;


/// Feed for Quake 3 server events
pub trait Quake3ServerEvents {


    /// Consumes this object, returning a `Stream` which yields Quake 3 server events
    fn events_stream(self) -> Result<Pin<Box<dyn Stream<Item=Result<Quake3Events>>>>>;
}
