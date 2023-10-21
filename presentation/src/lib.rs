//! Small crate to be a central point for presentation requisites.
//!
//! Simply shows the `Stream` of GameMatches as a Json

mod config;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::{Debug, Display};
pub use config::Config;

use std::io::Write;
use log::warn;
use model::report::GamesSummary;

/// IMPLEMENTATION NOTE: here we use our hand-crafter json instead of the one provided by the serde-json crate so we can better control the formatting of the output
///                      to match the exact specification + gain a bit of performance
pub fn to_json(config: &Config, games_summary_stream: GamesSummary, mut writer: impl Write) -> Result<(), Box<dyn std::error::Error>> {

    let mut write = |text: &str|
        writer.write(text.as_bytes())
            .map_err(|err| format!("presentation: to_json(): Error writing `GameMatchSummary` to the given `writer`: {err}"));

    let mut game_id = 1;
    let games_summary_stream = futures::executor::block_on_stream(games_summary_stream);
    write("{\n")?;
    for summary_result in games_summary_stream {
        match summary_result {
            Ok(summary) => {
                if (game_id > 1) {
                    write(",\n")?;
                }
                write(&format!("  \"game_{game_id}\": {{\n"))?;
                write(&format!("    \"total_kills\": {},\n", summary.total_kills))?;
                write(&format!("    \"players\": {},\n", serialize_set(&summary.players)))?;
                write(&format!("    \"kills\": {}", serialize_map("    ", &summary.kills)))?;

                // extended/optional field: means_of_death
                if let Some(means_of_death) = summary.means_of_death {
                    write(",\n")?;
                    write(&format!("    \"kills_by_means\": {}", serialize_map("    ", &means_of_death)))?;
                }
                // extended/optional field: game_reported_scores
                if let Some(game_reported_scores) = summary.game_reported_scores {
                    write(",\n")?;
                    write(&format!("    \"game_reported_scores\": {}", serialize_map("    ", &game_reported_scores)))?;
                }

                // extended/optional field: disconnected_players
                if let Some(disconnected_players) = summary.disconnected_players {
                    write(",\n")?;
                    write(&format!("    \"disconnected_players\": {}", serialize_vec("    ", &disconnected_players)))?;
                }

                write(&format!("\n  }}"))?;
            },

            Err(summary_err) => {
                let msg = format!("presentation: to_json(): Error in `games_summary_stream` while processing game_id {game_id}: {summary_err}");
                if config.log_errors {
                    warn!("{msg}");
                }
                if config.stop_on_errors {
                    return Err(Box::from(msg))
                }
            }
        }
        game_id += 1;
    }
    write("\n}")?;
    Ok(())
}

/// IMPLEMENTATION NOTE: this is left to demonstrate the flexibility of the architecture, allowing different implementations to better work with `Stream`,
///                      in case the application is enabled by Tokio.
///                      PS: some refactorings would be required for the [to_json()] and this function to not have repeated code.
pub async fn to_json_async(config: &Config, games_summary_stream: GamesSummary, writer: impl Write) -> Result<(), Box<dyn std::error::Error>> {
    todo!("Placeholder for an async implementation, that would be useful for async applications")
}

fn serialize_set(set: &BTreeSet<String>) -> String {
    let mut string = set.iter()
        .fold(String::from("["), |mut acc, element| {
            if acc.len() != 1 {
                acc.push_str(", ");
            }
            acc.push('"');
            acc.push_str(element);
            acc.push('"');
            acc
        });
    string.push(']');
    string
}

fn serialize_map<T: Display>(pre_ident: &str, map: &BTreeMap<String, T>) -> String {
    let mut string = map.iter()
        .fold(String::from("{\n  "), |mut acc, (key, value)| {
            if acc.len() != 4 {
                acc.push_str(",\n  ");
            }
            acc.push_str(pre_ident);
            acc.push_str(&format!("\"{key}\": {value}"));
            acc
        });
    string.push('\n' );
    string.push_str(pre_ident);
    string.push('}');
    string
}

fn serialize_vec(pre_ident: &str, vec: &Vec<(u32, String, i32)>) -> String {
    let mut string = vec.iter()
        .fold(String::from("[\n  "), |mut acc, (id, name, frags)| {
            if acc.len() != 4 {
                acc.push_str(",\n  ");
            }
            acc.push_str(pre_ident);
            acc.push_str(&format!("{{\"id\": {id}, \"name\": \"{name}\", \"frags\": {frags}}}"));
            acc
        });
    string.push('\n' );
    string.push_str(pre_ident);
    string.push(']');
    string
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use futures::stream;
    use serde_json::Error;
    use model::report::GameMatchSummary;

    #[test]
    fn single_standard_summary() {
        let summaries = vec![
            GameMatchSummary {
                total_kills: 45,
                players: BTreeSet::from(["Dono da bola".to_owned(), "Isgalamido".to_owned(), "Zeh".to_owned()]),
                kills: BTreeMap::from([
                    ("Dono da bola".to_owned(), 5),
                    ("Isgalamido".to_owned(), 18),
                    ("Zeh".to_owned(), 20),
                ]),
                means_of_death: None,
                game_reported_scores: None,
                disconnected_players: None,
            }
        ];
        assert_json(summaries)
    }

    #[test]
    fn single_complete_summary() {
        let summaries = vec![
            GameMatchSummary {
                total_kills: 45,
                players: BTreeSet::from(["Dono da bola".to_owned(), "Isgalamido".to_owned(), "Zeh".to_owned()]),
                kills: BTreeMap::from([
                    ("Dono da bola".to_owned(), 5),
                    ("Isgalamido".to_owned(), 18),
                    ("Zeh".to_owned(), 20),
                ]),
                means_of_death: Some(BTreeMap::from([
                    ("MOD_BRUTE_FORCE".to_owned(), 3),
                    ("MOD_PUNCH".to_owned(), 8),
                    ("MOD_NAIL_IN_THE_HEAD".to_owned(), 3),
                ])),
                game_reported_scores: Some(BTreeMap::from([
                    ("Dono da bola".to_owned(), 5),
                    ("Isgalamido".to_owned(), 18),
                    ("Zeh".to_owned(), 20),
                ])),
                disconnected_players: Some(vec![
                    (3, "Zeh Maneh".to_owned(), 2),
                    (7, "Alcantara".to_owned(), -3),
                ]),
            }
        ];
        assert_json(summaries)
    }

    #[test]
    fn double_standard_summaries() {
        let summaries = vec![
            GameMatchSummary {
                total_kills: 45,
                players: BTreeSet::from(["Dono da bola".to_owned(), "Isgalamido".to_owned(), "Zeh".to_owned()]),
                kills: BTreeMap::from([
                    ("Dono da bola".to_owned(), 5),
                    ("Isgalamido".to_owned(), 18),
                    ("Zeh".to_owned(), 20),
                ]),
                means_of_death: None,
                game_reported_scores: None,
                disconnected_players: None,
            },
            GameMatchSummary {
                total_kills: 45,
                players: BTreeSet::from(["Dono da bola".to_owned(), "Isgalamido".to_owned(), "Zeh".to_owned()]),
                kills: BTreeMap::from([
                    ("Dono da bola".to_owned(), 5),
                    ("Isgalamido".to_owned(), 18),
                    ("Zeh".to_owned(), 20),
                ]),
                means_of_death: None,
                game_reported_scores: None,
                disconnected_players: None,
            }
        ];
        assert_json(summaries)
    }

    fn assert_json(summaries: Vec<GameMatchSummary>) {
        let summaries = summaries.into_iter()
            .map(|summary| Ok(summary));
        let mut buffer = Cursor::new(Vec::new());
        to_json(
            &Config::default(),
            Box::pin(stream::iter(summaries)),
            &mut buffer
        ).expect("Failure in generating the json");
        let json_string = String::from_utf8(buffer.into_inner()).unwrap();
        print!("{json_string}");
        let json_error = validate_json(&json_string);
        assert!(json_error.is_none(), "The produced JSON is not valid: {:?}", json_error.unwrap());
    }

    fn validate_json(json_str: &str) -> Option<Error> {
        match serde_json::from_str::<serde_json::Value>(json_str) {
            Ok(_) => None,
            Err(err) => Some(err),
        }
    }}