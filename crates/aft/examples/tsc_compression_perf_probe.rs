//! Reproducible benchmark for compression of the fixed `tsc-generated-80k` workload:
//! 80,000 generated TypeScript diagnostics distributed across 40 files.
//!
//! Run from the workspace root with:
//! `cargo run --profile stage -p agent-file-tools --example tsc_compression_perf_probe`

use std::hint::black_box;
use std::time::Instant;

use aft::compress::{builtin_filters, compress_with_registry, toml_filter};
use sha2::{Digest, Sha256};

/// The `tsc-generated-80k` workload distributes its 80,000 diagnostics across 40 files.
const FILES: usize = 40;
/// Each file in the `tsc-generated-80k` workload contains 2,000 diagnostics.
const ERRORS_PER_FILE: usize = 2_000;
/// The `tsc-generated-80k` measurement runs 7 independent timing samples.
const SAMPLES: usize = 7;
/// Each `tsc-generated-80k` timing sample averages 5 compression calls to reduce noise.
const ITERATIONS: usize = 5;

fn main() {
    let output = corpus();
    let registry = toml_filter::build_registry(builtin_filters::ALL, None, None);
    let expected = compress_with_registry("tsc --noEmit", &output, &registry);

    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let started = Instant::now();
        for _ in 0..ITERATIONS {
            let actual = black_box(compress_with_registry(
                black_box("tsc --noEmit"),
                black_box(&output),
                black_box(&registry),
            ));
            assert_eq!(actual, expected, "compressed output changed during probe");
        }
        samples.push(started.elapsed().as_nanos() / ITERATIONS as u128);
    }
    samples.sort_unstable();

    println!("workload=tsc-generated-80k");
    println!("fixture_bytes={}", output.len());
    println!("files={FILES}");
    println!("diagnostics={}", FILES * ERRORS_PER_FILE);
    println!("iterations_per_sample={ITERATIONS}");
    println!("output_bytes={}", expected.len());
    println!("output_sha256={:x}", Sha256::digest(expected.as_bytes()));
    println!("samples_ns={samples:?}");
    println!("median_ns={}", samples[SAMPLES / 2]);
}

fn corpus() -> String {
    let mut output = String::with_capacity(FILES * ERRORS_PER_FILE * 100);
    for file in 0..FILES {
        for diagnostic in 0..ERRORS_PER_FILE {
            output.push_str(&format!(
                "src/generated/module_{file:02}.ts({diagnostic},17): error TS2322: Type 'string' is not assignable to type 'number'.\n"
            ));
        }
    }
    output.push_str(&format!(
        "Found {} errors in {FILES} files.\n",
        FILES * ERRORS_PER_FILE
    ));
    output
}
