mod config;
pub use config::*;


use model::{
    types::Result,
    quake3_events::Quake3Events,
};
use std::fmt::Debug;
use std::pin::Pin;
use futures::Stream;


/// Feed for Quake 3 server events
pub trait Quake3ServerEvents {


    /// Consumes this object, returning a `Stream` which yields Quake 3 server events
    fn events_stream(self) -> Result<Pin<Box<dyn Stream<Item=Quake3Events>>>>;
}
