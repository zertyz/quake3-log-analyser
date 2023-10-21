//! Resting place for BLL's [Config] & friends


use std::collections::HashSet;

/// Configuration to dictate the tunable behaviors of the Business Logic Layer
pub struct Config {

    /// Log::warn! of any errors that happen during log processing.\
    /// If either [Self::stop_on_feed_errors] or [Self::stop_on_event_model_violations] are set to false,
    /// feed, parsing or event structure errors won't cause the processors to stop.
    /// With this setting, you have the option to visualize any issues.
    pub log_issues: bool,

    /// If false, ignore any event data feed errors -- such as IO errors, parsing errors.\
    /// If true, causes the error to propagate and the processor to stop.
    pub stop_on_feed_errors: bool,

    /// If false, ignore any event model errors -- such as `Kill`s out of a started game.\
    /// If true, causes the error to propagate and the processor to stop.
    pub stop_on_event_model_violations: bool,

    /// What operations should be applied -- each with their own CPU & RAM resources needs
    pub processor_pipeline: HashSet<EventAnalyserOperations>,

}

/// The operations the Business Logic Layer may perform on the Quake3 Events feed
/// to aggregate into a summary to present to the user
#[derive(Debug, Eq, Hash, PartialEq)]
pub enum EventAnalyserOperations {
    MeansOfDeath,
    Kills,
    PlayerIdsAndNickNamesResolutions,
    GameReportedScores,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            log_issues: false,
            stop_on_feed_errors: false,
            stop_on_event_model_violations: false,
            processor_pipeline: HashSet::from([
                EventAnalyserOperations::Kills
            ])
        }
    }
}