//! Reproducible large-corpus acceptance harness for the staged callgraph build.
//!
//! Run twice in fresh processes so allocator residue cannot hide growth:
//! `cargo run -p agent-file-tools --example callgraph_bounded_harness -- 20000`
//! and `cargo run -p agent-file-tools --example callgraph_bounded_harness -- 40000`.

use aft::callgraph_store::{set_cold_build_phase_observer, CallGraphStore};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const GIB: u64 = 1024 * 1024 * 1024;

#[derive(Default)]
struct Samples {
    phase: &'static str,
    peak_bytes: BTreeMap<&'static str, u64>,
}

fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    if args.get(1).map(String::as_str) == Some("--run") {
        let root = PathBuf::from(args.get(2).expect("corpus path"));
        let storage = PathBuf::from(args.get(3).expect("storage path"));
        let file_count = args
            .get(4)
            .expect("file count")
            .parse()
            .expect("integer count");
        run_harness(&root, &storage, file_count);
        return;
    }
    let file_count = args
        .get(1)
        .map(String::as_str)
        .unwrap_or("20000")
        .parse::<usize>()
        .expect("file count must be an integer");
    assert!(
        file_count >= 20_000,
        "the acceptance corpus must contain >=20,000 files"
    );
    let corpus = tempfile::tempdir().expect("create corpus");
    let storage = tempfile::tempdir().expect("create storage");
    generate_corpus(corpus.path(), file_count);
    let output = Command::new(std::env::current_exe().expect("current executable"))
        .args([
            "--run",
            corpus.path().to_str().expect("utf8 corpus path"),
            storage.path().to_str().expect("utf8 storage path"),
            &file_count.to_string(),
        ])
        .output()
        .expect("run clean measurement child");
    assert!(
        output.status.success(),
        "measurement child failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    print!("{}", String::from_utf8_lossy(&output.stdout));
}

fn run_harness(corpus: &Path, storage: &Path, file_count: usize) {
    let mut files = aft::callgraph::walk_project_files(corpus).collect::<Vec<_>>();
    files.sort();
    assert_eq!(files.len(), file_count, "generated corpus file count");
    let samples = Arc::new(Mutex::new(Samples {
        phase: "enumeration",
        ..Samples::default()
    }));
    let sampling = install_rss_sampler(Arc::clone(&samples));
    let observer_samples = Arc::clone(&samples);
    set_cold_build_phase_observer(Some(Arc::new(move |phase| {
        observer_samples
            .lock()
            .expect("phase samples poisoned")
            .phase = phase;
    })));

    let cold_started = Instant::now();
    let (store, _stats) = CallGraphStore::cold_build_with_lease_chunked(
        storage.to_path_buf(),
        corpus.to_path_buf(),
        &files,
        256,
    )
    .expect("cold build");
    let cold_ms = cold_started.elapsed().as_millis();
    drop(store);
    let warm_store =
        CallGraphStore::open(storage.to_path_buf(), corpus.to_path_buf()).expect("open warm store");
    let warm_started = Instant::now();
    warm_store
        .cold_build_chunked(&files, 256)
        .expect("warm full build");
    let warm_ms = warm_started.elapsed().as_millis();
    drop(warm_store);

    sampling.store(true, Ordering::Release);
    set_cold_build_phase_observer(None);
    let samples = samples.lock().expect("phase samples poisoned");
    let full_peak = samples.peak_bytes.values().copied().max().unwrap_or(0);
    assert!(
        full_peak <= GIB,
        "cold build peak {} exceeds the 1.0 GiB cap",
        full_peak
    );
    println!(
        "{{\"files\":{file_count},\"cold_ms\":{cold_ms},\"warm_ms\":{warm_ms},\"warm_overhead_ratio\":{:.4},\"phase_peak_rss_bytes\":{:?}}}",
        warm_ms as f64 / cold_ms.max(1) as f64,
        samples.peak_bytes
    );
}

fn generate_corpus(root: &Path, count: usize) -> Vec<PathBuf> {
    let source_dir = root.join("src");
    std::fs::create_dir_all(&source_dir).expect("create source directory");
    let target = source_dir.join("target.ts");
    std::fs::write(&target, "export function target() { return 1; }\n").expect("write target");
    let mut files = Vec::with_capacity(count);
    files.push(target);
    for index in 1..count {
        let path = source_dir.join(format!("unit_{index:05}.ts"));
        std::fs::write(
            &path,
            format!(
                "import {{ target }} from './target';\nexport function unit_{index}() {{ return target(); }}\n"
            ),
        )
        .expect("write source");
        files.push(path);
    }
    files
}

fn install_rss_sampler(samples: Arc<Mutex<Samples>>) -> Arc<AtomicBool> {
    let stopped = Arc::new(AtomicBool::new(false));
    let stop = Arc::clone(&stopped);
    thread::spawn(move || {
        while !stop.load(Ordering::Acquire) {
            if let Some(bytes) = process_rss_bytes() {
                let mut samples = samples.lock().expect("phase samples poisoned");
                let phase = samples.phase;
                let peak = samples.peak_bytes.entry(phase).or_default();
                *peak = (*peak).max(bytes);
            }
            thread::sleep(Duration::from_millis(20));
        }
    });
    stopped
}

fn process_rss_bytes() -> Option<u64> {
    let output = Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()?;
    let kib = String::from_utf8(output.stdout)
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()?;
    Some(kib * 1024)
}
