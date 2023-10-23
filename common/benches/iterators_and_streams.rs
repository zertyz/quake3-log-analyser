//! Compares the performance of `Iterator`s and `Stream`s when used in a sync context,
//! to ensure if `Stream` offers acceptable performance in this scenario.
//!
//! `Stream`s in sync contexts are used along with `futures::executor::block_on_stream()`.
//!
//! # Analysis 2023-10-19
//!     1) Dismembering the event into its parts is 3200x faster with `str::split_n()` then with `Regex`. It is also simpler. We have a clear winner here.
//!     2) Due to the astonishing results above, the part 2 wasn't even done: split for the win!
//!

use criterion::{criterion_group, criterion_main, Criterion, black_box};
use futures::stream;
use once_cell::sync::Lazy;


/// Some sample log lines
const ELEMENTS: Lazy<[u32; 4096]> = Lazy::new(|| core::array::from_fn(|i| i as u32));


fn bench_u32(criterion: &mut Criterion) {

    let mut group = criterion.benchmark_group("Streams and Iterators on u32");

    let bench_id = "Iterator";
    group.bench_function(bench_id, |bencher| bencher.iter(|| {
        for n in ELEMENTS.into_iter() {
            black_box(n);
        }
    }));

    let bench_id = "Stream";
    group.bench_function(bench_id, |bencher| bencher.iter(|| {
        for n in futures::executor::block_on_stream(stream::iter(ELEMENTS.into_iter())) {
            black_box(n);
        }
    }));

    group.finish();
}

criterion_group!(benches, bench_u32);
criterion_main!(benches);