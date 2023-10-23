#![doc = include_str!("../README.md")]

mod config;
pub use config::*;

use common::types::Result;
use dal_api::Quake3ServerEvents;
use model::report::GamesSummary;
use std::sync::Arc;


pub trait SummaryLogicApi {

    /// Creates a new instance of the logic implementation
    fn new<IntoArcConfig: Into<Arc<Config>>>(config: IntoArcConfig) -> Self;

    /// Builds summaries of Quake3 games that comes from a `Stream` of game events,
    /// returning the data also in a `Stream`.\
    /// See [Config] for the options of how to compose the operations.
    fn summarize_games(&self, log_dao: Box<dyn Quake3ServerEvents>) -> Result<GamesSummary>;

}