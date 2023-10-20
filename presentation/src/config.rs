//! Resting place for Presentation's [Config] & friends

/// Configuration for the Presentation crate
pub struct Config {

    /// If false, ignore any errors and continue with the generation of the report
    pub stop_on_errors: bool,

    /// If true, logs any any errors found on the generation of the report
    pub log_errors: bool,

}

impl Default for Config {
    fn default() -> Self {
        Self {
            stop_on_errors: false,
            log_errors: true,
        }
    }
}