//! Compares the performance of `Iterator`s and `Stream`s when used in a sync context.
//!
//! `Stream`s in sync contexts are used along with `futures::executor::block_on_stream()`.
//!
//! # Analysis 2023-10-19
//!     1) `Iterator`s were measured as 50x faster than `Stream`s -- 1ns and 50ns per iteration, respectively
//!     2) Even so, `Stream`s are able to iterate 20 millions per second (1e9ns / 50ns = 2e7)
//!     3) When taking in account the message parsing times (20us, as measured by the `quake3-server-events` crate),
//!        we find that the time spent in `Stream`s is negligible -- with a 1/100 relation
//!     4) Due to the higher flexibility allowed by `Stream`s -- allowing async implementations -- this solution is
//!        justified.
//!

use criterion::{criterion_group, criterion_main, Criterion, black_box};
use futures::stream;
use once_cell::sync::Lazy;


fn bench_u32(criterion: &mut Criterion) {

    let mut group = criterion.benchmark_group("Streams and Iterators on u32");

    let bench_id = "Iterator";
    let mut iterator = (0..usize::MAX).into_iter();
    group.bench_function(bench_id, |bencher| bencher.iter(|| {
        black_box(iterator.next());
    }));

    let bench_id = "Stream";
    let mut stream = futures::executor::block_on_stream(stream::iter((0..usize::MAX).into_iter()));
    group.bench_function(bench_id, |bencher| bencher.iter(|| {
        black_box(stream.next());
    }));

    group.finish();
}

criterion_group!(benches, bench_u32);
criterion_main!(benches);