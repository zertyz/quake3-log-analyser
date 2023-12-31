//! Simple application to demonstrate the powers of the architecture:
//!
//! ================================================================
//! Generates a stream of Game Matches from Quake3 Server log files.
//! By default, reads from stdin unless '--log-file' is used
//! ================================================================
//!
//! USAGE:
//!     app [FLAGS] [OPTIONS]
//!
//! FLAGS:
//!         --debug       Logs to stderr the feed of Quake3ServerEvents, as passed to the summary logic
//!         --extended    Perform extended analysis on the log files, giving out an extended report as well
//!     -h, --help        Prints help information
//!         --pedantic    Considers all errors as fatal -- even the ones that might be ignored (such as an invalid log line)
//!     -V, --version     Prints version information
//!         --verbose     Outputs any non-fatal errors or inconsistencies in the events to stderr
//!
//! OPTIONS:
//!         --log-file <log-file>    Input file with Quake3 Server log messages
//!
//!
//! Explore some execution options:
//!  - ./target/release/app -help
//!  - ./target/release/app                                                       # reads the log lines from stdin
//!  - ./target/release/app --debug                                               # same as above, but also logs the parsed lines
//!  - ./target/release/app -extended --log-file '<path_to_quake3_log_file>'      # performs extra analysis and gives out a richer report
//!  - ./target/release/app -pedantic --log-file '<path_to_quake3_log_file>'      # stop on any error or inconsistency in the events
//!  - ./target/release/app -verbose  --log-file '<path_to_quake3_log_file>'      # continues on any non-fatal errors or inconsistencies in the events, but outputs them to stderr
//! 
//! To prove this application works with Streams of data of undefined size, run in bash:
//!  for i in {0..1000}; do cat 'bll/tests/resources/qgames_permissive.log'; done | time -v ./target/release/app --extended >/dev/null
//! 
//! To get the number of log lines from above, for lines per second calculation:
//!  for i in {0..1000}; do cat 'bll/tests/resources/qgames_permissive.log'; done | wc -l
//! 
//! Re-run the benchmark, this time without the extended logic extensions -- showing our logic pattern has zero-cost abstractions:
//!  for i in {0..1000}; do cat 'bll/tests/resources/qgames_permissive.log'; done | time -v ./target/release/app >/dev/null
//!
//! Interesting findings:
//!   1) By running with --verbose on the original log file, we get:
//!      2023-10-20T19:06:30.106Z WARN  [bll::summary_logic] Failed to process event #97: `LogParsingError` when processing log file '/home/luiz/tmp/quake3-log-analyser/bll/tests/resources/qgames_permissive.log' at line 97: EventParsingError { event_name: " 0", event_parsing_error: UnknownEventName }
//!      2023-10-20T19:06:30.106Z WARN  [presentation] presentation: to_json(): Error in `games_summary_stream` while processing game_id 2: Event #98: violated the event model: DoubleInit
//!   2) By adding the --extended flag, the messages grow to:
//!      2023-10-20T19:06:44.391Z WARN  [bll::summary_logic] Failed to process event #97: `LogParsingError` when processing log file '/home/luiz/tmp/quake3-log-analyser/bll/tests/resources/qgames_permissive.log' at line 97: EventParsingError { event_name: " 0", event_parsing_error: UnknownEventName }
//!      2023-10-20T19:06:44.391Z WARN  [presentation] presentation: to_json(): Error in `games_summary_stream` while processing game_id 2: Event #98: violated the event model: DoubleInit
//!      2023-10-20T19:06:44.391Z WARN  [presentation] presentation: to_json(): Error in `games_summary_stream` while processing game_id 3: Event #99: violated the event model: DoubleConnect
//!      2023-10-20T19:06:44.391Z WARN  [presentation] presentation: to_json(): Error in `games_summary_stream` while processing game_id 4: Event #115: Player id: 0, name: "Isgalamido" is already registered
//!   3) The --extended flag includes the scores reported by the game. None of them matches the scores calculated by this application.
//!      After a thorough analysis, the log file contents are to blame.

mod command_line;

use bll_api::SummaryLogicApi;
use std::{
    borrow::Cow,
    collections::HashSet,
    io::BufWriter,
    sync::Arc,
};


/// Buffer to allow efficient output operations
const OUTPUT_BUFFER_SIZE: usize = 1024 * 1024;


fn main() -> Result<(), Box<dyn std::error::Error>> {

    // start the logger
    simple_logger::SimpleLogger::new().with_utc_timestamps().init().unwrap_or_else(|_| eprintln!("--> LOGGER WAS ALREADY STARTED"));

    let command_line_options = command_line::parse_from_args();

    let dal_implementation = match command_line_options.log_file {
        Some(log_file) => dal_api::Quake3ServerEventsImplementations::SyncLogFileReader(dal_api::FileReaderInfo { log_file_path: Cow::Owned(log_file) }),
        None => dal_api::Quake3ServerEventsImplementations::StdinReader,
    };
    let dal_config = Arc::new(dal_api::Config {
        debug: command_line_options.debug,
        ..dal_api::Config::default()
    });
    let logic_config = bll_api::Config {
        log_issues: command_line_options.verbose,
        stop_on_feed_errors: command_line_options.pedantic,
        stop_on_event_model_violations: command_line_options.pedantic,
        processor_pipeline: if command_line_options.extended {
            HashSet::from([
                bll_api::EventAnalyserOperations::MeansOfDeath,
                bll_api::EventAnalyserOperations::Kills,
                bll_api::EventAnalyserOperations::PlayerIdsAndNickNamesResolutions,
                bll_api::EventAnalyserOperations::GameReportedScores,
            ])
        } else {
            HashSet::from([
                bll_api::EventAnalyserOperations::Kills,
            ])
        },
        ..bll_api::Config::default()
    };
    let presentation_config = presentation::Config {
        log_errors: command_line_options.verbose,
        stop_on_errors: command_line_options.pedantic,
        ..presentation::Config::default()
    };
    let presentation_writer = BufWriter::with_capacity(OUTPUT_BUFFER_SIZE, std::io::stdout());


    let log_dao = dal::factory::instantiate_log_dao(dal_implementation, dal_config);
    let logic = bll::SummaryLogic::new(logic_config);
    let summaries_stream = logic.summarize_games(log_dao)?;
    presentation::to_json(&presentation_config, summaries_stream, presentation_writer)?;

    Ok(())
}
