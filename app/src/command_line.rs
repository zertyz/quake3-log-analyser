//! Configs that are gathered from the command line -- see [CommandLineOptions]

use structopt::StructOpt;


/// Command-line options
#[derive(Debug,StructOpt)]
#[structopt(about = "
================================================================
Generates a stream of Game Matches from Quake3 Server log files.
By default, reads from the file at './qgames.log'
================================================================
")]
pub struct CommandLineOptions {

    // FLAGS
    ////////

    /// Outputs any non-fatal errors or inconsistencies in the events to stderr
    #[structopt(long)]
    pub verbose: bool,

    /// Perform extended analysis on the log files, giving out an extended report as well
    #[structopt(long)]
    pub extended: bool,

    /// Considers all errors as fatal -- even the ones that might be ignored (such as an invalid log line)
    #[structopt(long)]
    pub pedantic: bool,


    // OPTIONS
    //////////


    /// Input file with Quake3 Server log messages
    #[structopt(long)]
    pub log_file: Option<String>,

}

pub fn parse_from_args() -> CommandLineOptions {
    fill_in_defaults(CommandLineOptions::from_args())
}

fn fill_in_defaults(command_line_options: CommandLineOptions) -> CommandLineOptions {
    // no defaults to fill in yet
    command_line_options
}