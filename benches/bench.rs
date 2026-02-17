use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};

use crate::{
    consts::{
        BIG_OLE_DATA_LINE, COMMENT_LINE, DATA_LINE, EMPTY_LINE, EVENT_LINE, ID_LINE, NO_SPACE_LINE,
        NO_VALUE_LINE, generate_one_of_each,
    },
    event_stream::{
        load_chunks, load_line_aligned_chunks, run_eventsource_stream, run_sseer,
        run_sseer_bytes_only,
    },
};

// eventsource-stream parser, it isn't public but I wanted to bench against it
mod es_parser;

pub(crate) mod consts;
pub(crate) mod event_stream;

/// Single-line parser comparison
fn bench_parse_line(c: &mut Criterion) {
    let mut group = c.benchmark_group("parse_line");

    let lines: &[(&str, &[u8])] = &[
        ("data_field", DATA_LINE),
        ("comment", COMMENT_LINE),
        ("event_field", EVENT_LINE),
        ("id_field", ID_LINE),
        ("empty_line", EMPTY_LINE),
        ("no_value", NO_VALUE_LINE),
        ("no_space", NO_SPACE_LINE),
        ("big_data_line", BIG_OLE_DATA_LINE),
    ];

    for &(name, line) in lines {
        group.bench_with_input(BenchmarkId::new("sseer", name), line, |b, input| {
            b.iter(|| {
                let _ = black_box(sseer::parser::parse_line(black_box(input)));
            });
        });

        let line_str = std::str::from_utf8(line).unwrap();
        group.bench_with_input(
            BenchmarkId::new("eventsource_stream", name),
            line_str,
            |b, input| {
                b.iter(|| {
                    let _ = black_box(es_parser::line(black_box(input)));
                });
            },
        );
    }

    group.finish();
}

fn bench_event_stream(c: &mut Criterion) {
    let mixed_raw = include_bytes!("../bench_data/mixed.bin");
    let ai_raw = include_bytes!("../bench_data/ai_stream.bin");

    let mixed_chunks = load_chunks(mixed_raw);
    let ai_chunks = load_chunks(ai_raw);

    let evenish_distribution = load_chunks(&generate_one_of_each(128));

    let mixed_aligned = load_line_aligned_chunks(mixed_raw);
    let ai_line_aligned = load_line_aligned_chunks(ai_raw);

    let mut group = c.benchmark_group("event_stream");

    for (name, alignment, chunks) in [
        ("mixed", "unaligned", &mixed_chunks),
        ("ai_stream", "unaligned", &ai_chunks),
        ("evenish_distribution", "unaligned", &evenish_distribution),
        ("ai_stream", "line-aligned", &ai_line_aligned),
        ("mixed", "line-aligned", &mixed_aligned),
    ] {
        let name = format!("{name}_{alignment}");
        group.bench_with_input(BenchmarkId::new("sseer", &name), chunks, |b, chunks| {
            b.iter(|| run_sseer(chunks));
        });

        group.bench_with_input(
            BenchmarkId::new("sseer_bytes_only", &name),
            chunks,
            |b, chunks| {
                b.iter(|| run_sseer_bytes_only(chunks));
            },
        );

        group.bench_with_input(
            BenchmarkId::new("eventsource_stream", &name),
            chunks,
            |b, chunks| {
                b.iter(|| run_eventsource_stream(chunks));
            },
        );
    }

    group.finish();
}

criterion_group!(benches, bench_parse_line, bench_event_stream);
criterion_main!(benches);
