use bdj_search_core::index::{IndexBuilder, MmapIndex};
use bdj_search_core::query::Parser;
use bdj_search_core::search::matcher::SubstringMatcher;
use bdj_search_core::search::Engine;
use criterion::{black_box, criterion_group, criterion_main, Criterion};
use std::io::BufWriter;
use tempfile::NamedTempFile;

fn bench_parser(c: &mut Criterion) {
    let query = "michael jackson (house | techno) ext:wav tam:>10mb !remix";
    c.bench_function("query_parser_complex", |b| {
        b.iter(|| Parser::parse(black_box(query)))
    });
}

fn bench_simd_matcher(c: &mut Criterion) {
    let matcher = SubstringMatcher::new("billie");
    let haystack = b"01. Michael Jackson - Billie Jean (Original 12 Inch Club Mix).wav";

    c.bench_function("simd_matcher_ascii", |b| {
        b.iter(|| matcher.matches(black_box(haystack), false))
    });
}

fn bench_search_refinement(c: &mut Criterion) {
    let count = 50_000usize;
    let mut builder = IndexBuilder::with_capacity(count);

    let artists = ["Michael", "Madonna", "Daft", "David", "Calvin", "Tiësto", "Fisher"];
    let titles = ["Billie", "Titanium", "Levels", "Animals", "Ferrari", "Strobe", "Gecko"];

    for i in 0..count {
        let name = format!(
            "{} - {} (Original Mix).wav",
            artists[i % artists.len()],
            titles[(i / artists.len()) % titles.len()]
        );
        builder.add_entry(u32::MAX, &name, false, false, false, 0, (i as u64) * 1024, 1700000000, 1700000000);
    }

    let temp_file = NamedTempFile::new().unwrap();
    let path = temp_file.path();
    let file = std::fs::File::create(path).unwrap();
    let mut writer = BufWriter::new(file);
    builder.write_to(&mut writer).unwrap();
    drop(writer);

    let mmap = MmapIndex::open(path).unwrap();
    let view = mmap.view().unwrap();
    let engine = Engine::new();

    // Warm up initial query
    engine.search(&view, "m", 0, true);

    c.bench_function("search_refinement_sub_8ms", |b| {
        b.iter(|| {
            engine.search(&view, black_box("mi"), 0, true);
        })
    });
}

criterion_group!(benches, bench_parser, bench_simd_matcher, bench_search_refinement);
criterion_main!(benches);
