mod command_line;

use std::collections::HashSet;
use std::io::BufWriter;
use std::sync::Arc;

/// Buffer to allow efficient output operations
const OUTPUT_BUFFER_SIZE: usize = 1024 * 1024;

fn main() -> Result<(), Box<dyn std::error::Error>> {

    let command_line_options = command_line::parse_from_args();

    let dal_config = dal_api::Config {
        ..dal_api::Config::default()
    };
    let logic_config = bll::Config {
        log_issues: command_line_options.verbose,
        stop_on_feed_errors: command_line_options.pedantic,
        stop_on_event_model_violations: command_line_options.pedantic,
        processor_pipeline: if command_line_options.extended {
            HashSet::from([
                bll::EventAnalyserOperations::Kills,
                bll::EventAnalyserOperations::PlayerIdsAndNickNamesResolutions,
                bll::EventAnalyserOperations::GameReportedScores,
            ])
        } else {
            HashSet::from([
                bll::EventAnalyserOperations::Kills,
            ])
        },
        ..bll::Config::default()
    };
    let presentation_config = presentation::Config {
        log_errors: command_line_options.verbose,
        stop_on_errors: command_line_options.pedantic,
        ..presentation::Config::default()
    };
    let presentation_writer = BufWriter::with_capacity(1024*1024, std::io::stdout());


    let log_dao = dal::factory::instantiate_log_dao(&dal_config, command_line_options.log_file.as_ref().unwrap());
    let summaries_stream = bll::summary::summarize_games(Arc::new(logic_config), log_dao)?;
    presentation::to_json(&presentation_config, summaries_stream, presentation_writer)?;

    // get a file
    // build a dal
    // get the logic
    // generate the report
    // output it

    Ok(())
}
