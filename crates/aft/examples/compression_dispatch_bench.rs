use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::hint::black_box;
use std::time::{Duration, Instant};

use aft::compress::{builtin_filters, compress_with_registry, toml_filter};

const SIZES: &[usize] = &[100 * 1024, 1024 * 1024, 5 * 1024 * 1024];
const RUNS: usize = 5;

struct Case {
    name: &'static str,
    command: &'static str,
    fragment: &'static str,
}

fn main() {
    let hashes_only = std::env::args().any(|arg| arg == "--hashes");
    let registry = toml_filter::build_registry(builtin_filters::ALL, None, None);
    let cases = [
        Case {
            name: "cargo-build",
            command: "cargo build",
            fragment: "\u{1b}[1m\u{1b}[32m   Compiling\u{1b}[0m dependency_name v0.12.3\nwarning: unused variable: `item`\n  --> src/lib.rs:42:9\n   |\n42 |     let item = 1;\n   |         ^^^^ help: prefix it with an underscore: `_item`\n\n",
        },
        Case {
            name: "vitest-sniffer",
            command: "npm test",
            fragment: "\u{1b}[32m ✓\u{1b}[0m src/example.test.ts > suite > handles a production-shaped case 12ms\n Test Files  1 passed (1)\n      Tests  1 passed (1)\n   Duration  1.25s (transform 80ms, setup 20ms, collect 100ms, tests 12ms)\n",
        },
        Case {
            name: "git-log",
            command: "git log",
            fragment: "commit 0123456789abcdef0123456789abcdef01234567\nAuthor: Example Developer <dev@example.com>\nDate:   Fri Aug 14 12:00:00 2026 +0000\n\n    Improve compression throughput without changing output\n\n",
        },
        Case {
            name: "npm-package",
            command: "npm install",
            fragment: "\u{1b}[2K\u{1b}[1Gnpm WARN deprecated old-package@1.0.0: package is deprecated\nadded 10 packages, and audited 11 packages in 2s\nfound 0 vulnerabilities\n",
        },
        Case {
            name: "docker-toml",
            command: "docker build .",
            fragment: "\u{1b}[2K#12 [builder 3/8] RUN cargo build --release\n#12 DONE 0.8s\n#13 transferring context: 1.2MB done\napplication diagnostic line retained by the filter\n",
        },
        Case {
            name: "generic-ansi",
            command: "custom-build-runner --verbose",
            fragment: "\u{1b}[32minfo\u{1b}[0m compiling component alpha\n\u{1b}[33mwarning\u{1b}[0m generated source changed\nprogress item completed successfully\n",
        },
    ];

    for case in cases {
        for &size in SIZES {
            let input = corpus(case.fragment, size);
            let compressed = compress_with_registry(case.command, &input, &registry);
            if hashes_only {
                let mut hasher = DefaultHasher::new();
                compressed.hash(&mut hasher);
                println!(
                    "{}\t{}\t{}\t{:016x}",
                    case.name,
                    input.len(),
                    compressed.len(),
                    hasher.finish()
                );
                continue;
            }

            let iterations = iterations_for(size);
            let mut samples = Vec::with_capacity(RUNS);
            for _ in 0..RUNS {
                let started = Instant::now();
                for _ in 0..iterations {
                    black_box(compress_with_registry(
                        black_box(case.command),
                        black_box(&input),
                        black_box(&registry),
                    ));
                }
                samples.push(started.elapsed() / iterations as u32);
            }
            samples.sort_unstable();
            println!(
                "{}\t{}\t{}\t{}",
                case.name,
                input.len(),
                iterations,
                micros(samples[RUNS / 2])
            );
        }
    }
}

fn corpus(fragment: &str, size: usize) -> String {
    let mut result = String::with_capacity(size + fragment.len());
    while result.len() < size {
        result.push_str(fragment);
    }
    result.truncate(size);
    result
}

fn iterations_for(size: usize) -> usize {
    match size {
        0..=200_000 => 100,
        200_001..=2_000_000 => 20,
        _ => 5,
    }
}

fn micros(duration: Duration) -> String {
    format!("{:.3}", duration.as_secs_f64() * 1_000.0)
}
