//! Include README

mod config;
pub use config::*;


use common::types::Result;
use model::quake3_events::Quake3Events;
use std::pin::Pin;
use futures::Stream;


/// Feed for Quake 3 server events
pub trait Quake3ServerEvents {

    /// Consumes this object, returning a `Stream` which yields our version of the [Quake3Events]
    fn events_stream(self: Box<Self>) -> Result<Pin<Box<dyn Stream<Item=Quake3Events<'static>>>>>;
}
