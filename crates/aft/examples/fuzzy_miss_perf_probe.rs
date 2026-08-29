//! Reproducible benchmark for a failed fuzzy edit match in generated ASCII source.
//!
//! Run from the workspace root with:
//! `cargo run --release -p agent-file-tools --example fuzzy_miss_perf_probe`

use aft::fuzzy_match::find_all_fuzzy;
use std::hint::black_box;
use std::time::Instant;

fn main() {
    let line = "export const generated_existing_symbol = generated_value_with_padding_padding_padding_padding_padding_padding_padding_padding_padding;\n";
    let lines = 32_768usize;
    let samples = 11usize;
    let iterations = 10usize;
    let needle =
        "export const requested_missing_symbol = missing_value;\nreturn requested_missing_symbol;";
    let source = line.repeat(lines);

    let warm = find_all_fuzzy(&source, needle);
    assert!(warm.is_empty());
    let output = canonical_output(&warm);
    std::fs::create_dir_all("target").expect("create target directory");
    std::fs::write("target/fuzzy-miss-output.txt", output.as_bytes())
        .expect("write canonical output");

    let mut elapsed_ns = Vec::with_capacity(samples);
    for _ in 0..samples {
        let started = Instant::now();
        for _ in 0..iterations {
            let matches = find_all_fuzzy(black_box(&source), black_box(needle));
            assert!(matches.is_empty());
            black_box(matches);
        }
        elapsed_ns.push(started.elapsed().as_nanos() / iterations as u128);
    }
    elapsed_ns.sort_unstable();

    println!("fixture_bytes={}", source.len());
    println!("fixture_lines={lines}");
    println!("needle_bytes={}", needle.len());
    println!("samples={samples}");
    println!("iterations_per_sample={iterations}");
    println!("samples_ns={elapsed_ns:?}");
    println!("median_ns={}", elapsed_ns[samples / 2]);
    println!("min_ns={}", elapsed_ns[0]);
    println!("max_ns={}", elapsed_ns[samples - 1]);
}

fn canonical_output(matches: &[aft::fuzzy_match::FuzzyMatch]) -> String {
    let mut output = format!("matches={}\n", matches.len());
    for found in matches {
        output.push_str(&format!(
            "{}:{}:{}\n",
            found.byte_start, found.byte_len, found.pass
        ));
    }
    output
}
