//! Compares the performance of Regular Experssions and `std::str::*` functions to parse Quake3 server log files
//! to help us decide on the implementation strategy, which, for this project should be a reasonable balance
//! between performance and simplicity.
//!
//! The task of parsing a log file has been broken down in 2 parts for the measurements here:
//!   1) Identifying the event type
//!   2) Extracting the event data
//!
//! # Analysis 2023-10-19
//!     1) Dismembering the event into its parts is 3200x faster with `str::split_n()` than with `Regex`. It is also simpler. We have a clear winner here.
//!     2) Due to the astonishing results above, the part 2 wasn't even done: split for the win!
//!
//!   `reactive-mutiny`'s Atomic is the winner on all tests, for a variety of Intel, AMD and ARM cpus -- sometimes winning by 2x.
//!
//! Out of the results here, it was decided that the `reactive-mutiny`'s Atomic Channel will be used instead of Tokio's
//!

use criterion::{criterion_group, criterion_main, Criterion, BenchmarkGroup, measurement::WallTime, black_box};
use once_cell::sync::Lazy;
use regex::Regex;

const LOG_EXERPT: &[&str] = &[
    r#"  0:37 ------------------------------------------------------------"#,
    r#" 80:37 ------------------------------------------------------------"#,
    r#"980:37 ------------------------------------------------------------"#,
    r#" 1:47 InitGame: \sv_floodProtect\1\sv_maxPing\0\sv_minPing\0\sv_maxRate\10000\sv_minRate\0\sv_hostname\Code Miner Server\g_gametype\0\sv_privateClients\2\sv_maxclients\16\sv_allowDownload\0\bot_minplayers\0\dmflags\0\fraglimit\20\timelimit\15\g_maxGameClients\0\capturelimit\8\version\ioq3 1.36 linux-x86_64 Apr 12 2009\protocol\68\mapname\q3dm17\gamename\baseq3\g_needpass\0"#,
    r#" 2:33 InitGame: \capturelimit\8\g_maxGameClients\0\timelimit\15\fraglimit\20\dmflags\0\bot_minplayers\0\sv_allowDownload\0\sv_maxclients\16\sv_privateClients\2\g_gametype\4\sv_hostname\Code Miner Server\sv_minRate\0\sv_maxRate\10000\sv_minPing\0\sv_maxPing\0\sv_floodProtect\1\version\ioq3 1.36 linux-x86_64 Apr 12 2009\protocol\68\mapname\Q3TOURNEY6_CTF\gamename\baseq3\g_needpass\0"#,
    r#" 2:33 ClientConnect: 2"#,
    r#"2:33 ClientUserinfoChanged: 2 n\Isgalamido\t\1\model\uriel/zael\hmodel\uriel/zael\g_redteam\\g_blueteam\\c1\5\c2\5\hc\100\w\0\l\0\tt\0\tl\0"#,
    r#" 2:33 ClientBegin: 2"#,
    r#" 2:33 ClientDisconnect: 2"#,
    r#" 2:36 Item: 2 ammo_rockets"#,
    r#"981:26 say: Isgalamido: team blue"#,
    r#"20:54 Kill: 1022 2 22: <world> killed Isgalamido by MOD_TRIGGER_HURT"#,
    r#"10:12 Exit: Capturelimit hit."#,
    r#"10:12 red:8  blue:6"#,
    r#"10:12 score: 77  ping: 3  client: 2 Isgalamido"#,
    r#"10:12 score: -77  ping: 3  client: 5 Dono da Bola"#,
    r#"10:28 ShutdownGame:"#,
];


// Our implementation candidates
////////////////////////////////
// 1) `*_event_identification()` are the functions that will identify the event in the log line and will split it into the `(time, event_name, data)` components
// 2) `*_data_extraction()` are the functions that will parse the `data` component of some events

const LOG_PARSING_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r#"^ *(?P<time>\d{1,3}:\d{2}) (?P<event_name>[^:]*):? ?(?P<data>.*)$"#)
        .expect("LOG_PARSING_REGEX compilation failed")
});
fn regex_event_identification(log_line: &str) -> (&str, &str, &str) {
    LOG_PARSING_REGEX.captures(log_line)
        .map(|captures| {
            (
                captures.name("time")
                    .expect("regex_event_identification(): Couldn't extract `time` from the log line")
                    .as_str(),
                captures.name("event_name")
                    .expect("regex_event_identification(): Couldn't extract `event_name` from the log line")
                    .as_str(),
                captures.name("data")
                    .expect("regex_event_identification(): Couldn't extract `data` from the log line")
                    .as_str(),
            )
        })
        .expect("regex_event_identification(): Failed to apply regex to the `log_line`")
}

fn split_event_identification(log_line: &str) -> (&str, &str, &str) {
    let mut parts = log_line.trim_start().splitn(3, " ");
    (
        parts.next()
            .expect("split_event_identification(): Couldn't extract `time` from the log line")
            .trim_end_matches(":"),
        parts.next()
            .expect("split_event_identification(): Couldn't extract `event_name` from the log line"),
        parts.next()
            .unwrap_or(""),
    )
}

/// Benchmarks the event identification strategies
fn bench_event_identification(criterion: &mut Criterion) {

    let mut group = criterion.benchmark_group("Event Identification");

    let bench_id = "regex_event_identification()";
    group.bench_function(bench_id, |bencher| bencher.iter(|| {
        for log_line in LOG_EXERPT {
            black_box(regex_event_identification(log_line));
        }
    }));

    let bench_id = "split_event_identification()";
    group.bench_function(bench_id, |bencher| bencher.iter(|| {
        for log_line in LOG_EXERPT {
            black_box(split_event_identification(log_line));
        }
    }));

    group.finish();
}

criterion_group!(benches, bench_event_identification);
criterion_main!(benches);