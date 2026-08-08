//! Offline measurement harness for one-file incremental callgraph refreshes.
//!
//! Run against a copy of a production store:
//! `AFT_CALLGRAPH_REFRESH_STORE=/path/to/<root-key> AFT_CALLGRAPH_REFRESH_ROOT=/path/to/project cargo test -p agent-file-tools --test callgraph_refresh_bench -- --ignored --nocapture`
//! Set `AFT_CALLGRAPH_REFRESH_FILE` to choose a project-relative file. Without
//! these variables the harness builds a synthetic store with a large fixture.

use aft::callgraph_store::{CallGraphStore, RefreshFilesProfile};
use rusqlite::{backup::Backup, params, Connection, OpenFlags};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tempfile::TempDir;

#[test]
#[ignore = "offline benchmark copies or builds a large callgraph store"]
fn bench_refresh_files_on_store_copy() {
    let temp = tempfile::tempdir().expect("benchmark temp dir");
    let (store, changed_file) = match (
        std::env::var_os("AFT_CALLGRAPH_REFRESH_STORE"),
        std::env::var_os("AFT_CALLGRAPH_REFRESH_ROOT"),
    ) {
        (Some(source_store), Some(project_root)) => open_production_store_copy(
            &temp,
            Path::new(&source_store),
            PathBuf::from(project_root),
            std::env::var_os("AFT_CALLGRAPH_REFRESH_FILE").map(PathBuf::from),
        ),
        (None, None) => build_synthetic_store(&temp),
        _ => panic!(
            "AFT_CALLGRAPH_REFRESH_STORE and AFT_CALLGRAPH_REFRESH_ROOT must be set together"
        ),
    };

    report_query_plans(store.sqlite_path());
    let (stats, profile) = store
        .refresh_files_profiled(std::slice::from_ref(&changed_file))
        .expect("profile one-file refresh");
    eprintln!("refresh_files stats: {stats:?}");
    eprintln!("refresh_files phases: {}", profile.report());
    report_dominant_phase(&profile);
}

fn open_production_store_copy(
    temp: &TempDir,
    source_store: &Path,
    project_root: PathBuf,
    requested_file: Option<PathBuf>,
) -> (CallGraphStore, PathBuf) {
    let pointer = fs::read_dir(source_store)
        .expect("read source store directory")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.extension().and_then(|ext| ext.to_str()) == Some("current"))
        .expect("source store has a current pointer");
    let source_db = source_store.join(
        fs::read_to_string(pointer)
            .expect("read current pointer")
            .trim(),
    );
    let copied_store = temp.path().join("store-copy");
    fs::create_dir_all(&copied_store).expect("create copied store directory");
    let project_key = aft::search_index::artifact_cache_key(&project_root);
    let copied_db = copied_store.join(format!("{project_key}.sqlite"));
    sqlite_backup(&source_db, &copied_db);

    let rel_path = requested_file.unwrap_or_else(|| select_fixture_file(&copied_db));
    let changed_file = if rel_path.is_absolute() {
        rel_path
    } else {
        project_root.join(rel_path)
    };
    assert!(
        changed_file.is_file(),
        "benchmark file must exist: {}",
        changed_file.display()
    );
    force_stale(&copied_db, &project_root, &changed_file);

    let store = CallGraphStore::open(copied_store, project_root).expect("open copied store");
    (store, changed_file)
}

fn sqlite_backup(source_db: &Path, copied_db: &Path) {
    let source = Connection::open_with_flags(
        source_db,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_URI,
    )
    .expect("open source store read-only");
    let mut destination = Connection::open(copied_db).expect("create copied database");
    let backup = Backup::new(&source, &mut destination).expect("start SQLite online backup");
    backup
        .run_to_completion(256, Duration::from_millis(5), None)
        .expect("copy SQLite store consistently");
}

fn select_fixture_file(db: &Path) -> PathBuf {
    let conn = Connection::open(db).expect("open copied store for fixture selection");
    conn.query_row(
        "SELECT path FROM files WHERE path LIKE '%.ts' ORDER BY size DESC LIMIT 1",
        [],
        |row| row.get::<_, String>(0),
    )
    .map(PathBuf::from)
    .expect("copied store has a TypeScript file")
}

fn force_stale(db: &Path, project_root: &Path, changed_file: &Path) {
    let rel_path = changed_file
        .strip_prefix(project_root)
        .expect("benchmark file belongs to project root")
        .to_string_lossy()
        .replace('\\', "/");
    let conn = Connection::open(db).expect("open copied store for stale marker");
    let changed = conn
        .execute(
            "UPDATE files SET content_hash = ?1, mtime_ns = 0 WHERE path = ?2",
            params!["benchmark-forced-stale", rel_path],
        )
        .expect("force copied row stale");
    assert_eq!(changed, 1, "benchmark file must already be indexed");
}

fn build_synthetic_store(temp: &TempDir) -> (CallGraphStore, PathBuf) {
    let project_root = temp.path().join("synthetic-project");
    let src = project_root.join("src");
    fs::create_dir_all(&src).expect("create synthetic fixture");

    let large_file = src.join("large.ts");
    let mut large_source = String::from("export function symbol0() {\n  let total = 0;\n");
    for index in 0..596 {
        large_source.push_str(&format!("  total += {index};\n"));
    }
    large_source.push_str("  return total;\n}\n");
    fs::write(&large_file, large_source).expect("write large synthetic file");

    let mut files = vec![large_file.clone()];
    for index in 0..1_000 {
        let path = src.join(format!("consumer{index}.ts"));
        fs::write(
            &path,
            format!(
                "import {{ symbol0 }} from './large';\nexport function consumer{index}() {{ return symbol0(); }}\n"
            ),
        )
        .expect("write synthetic consumer");
        files.push(path);
    }

    let store = CallGraphStore::open(temp.path().join("synthetic-store"), project_root)
        .expect("open synthetic store");
    store.cold_build(&files).expect("build synthetic store");
    let changed = fs::read_to_string(&large_file)
        .expect("read synthetic changed file")
        .replace("total += 595", "total += 596");
    fs::write(&large_file, changed).expect("make synthetic file stale");
    (store, large_file)
}

fn report_query_plans(db: &Path) {
    let conn = Connection::open(db).expect("open copied store for query plans");
    for (name, sql) in [
        (
            "dependent_refs",
            "EXPLAIN QUERY PLAN SELECT DISTINCT r.ref_id FROM refs r WHERE r.caller_file IN (SELECT file_path FROM file_dependencies WHERE dep_file = 'src/large.ts') OR r.target_file = 'src/large.ts'",
        ),
        (
            "delete_edges_by_ref",
            "EXPLAIN QUERY PLAN DELETE FROM edges WHERE ref_id = 'benchmark-ref'",
        ),
        (
            "method_refs_by_caller",
            "EXPLAIN QUERY PLAN SELECT r.ref_id FROM refs r JOIN files f ON f.path = r.caller_file JOIN nodes n ON n.id = r.caller_node WHERE r.kind = 'call' AND r.status = 'unresolved' AND r.caller_file = 'src/large.ts'",
        ),
        (
            "all_index_nodes",
            "EXPLAIN QUERY PLAN SELECT file_path, id, name, scoped_name, exported, is_default_export FROM nodes",
        ),
    ] {
        let mut stmt = conn.prepare(sql).expect("prepare query plan");
        let plan = stmt
            .query_map([], |row| row.get::<_, String>(3))
            .expect("run query plan")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect query plan")
            .join(" | ");
        eprintln!("query_plan[{name}]: {plan}");
    }
}

fn report_dominant_phase(profile: &RefreshFilesProfile) {
    let phases = [
        ("parse", profile.parse),
        ("dependency_selection", profile.dependency_selection),
        ("row_deletes", profile.row_deletes),
        ("row_inserts", profile.row_inserts),
        ("dependent_parse", profile.dependent_parse),
        ("index_load", profile.index_load),
        ("ref_resolution", profile.ref_resolution),
        ("method_dispatch", profile.method_dispatch),
        ("commit", profile.commit),
    ];
    let (name, elapsed) = phases
        .into_iter()
        .max_by_key(|(_, elapsed)| *elapsed)
        .expect("profile has phases");
    eprintln!("refresh_files hot_loop: {name} ({}ms)", elapsed.as_millis());
}

#[derive(Debug)]
struct WriteAmplificationMeasurement {
    directory_delta_bytes: u64,
    output_blocks: u64,
}

/// Compare the shipped configuration with the legacy full-scan configuration,
/// which scans 1,000 pages and skips row-difference checks. Keep this test ignored
/// because it performs 50 edits for an offline write-amplification measurement.
#[test]
#[ignore = "offline write-amplification measurement"]
fn measure_write_amplification_ab() {
    let baseline = run_write_amplification_sequence(true);
    let optimized = run_write_amplification_sequence(false);
    let byte_ratio = if baseline.directory_delta_bytes == 0 {
        None
    } else {
        Some(optimized.directory_delta_bytes as f64 / baseline.directory_delta_bytes as f64)
    };
    let block_ratio = if baseline.output_blocks == 0 {
        None
    } else {
        Some(optimized.output_blocks as f64 / baseline.output_blocks as f64)
    };
    eprintln!(
        "write_amplification_ab baseline={{directory_delta_bytes:{}, output_blocks:{}}} optimized={{directory_delta_bytes:{}, output_blocks:{}}} ratios={{bytes:{byte_ratio:?}, blocks:{block_ratio:?}}}",
        baseline.directory_delta_bytes,
        baseline.output_blocks,
        optimized.directory_delta_bytes,
        optimized.output_blocks,
    );
}

fn run_write_amplification_sequence(baseline: bool) -> WriteAmplificationMeasurement {
    let temp = tempfile::tempdir().expect("measurement temp dir");
    let project_root = temp.path().join("project");
    let store_dir = project_root.join(".store-write-amp");
    fs::create_dir_all(&project_root).expect("create measurement project");
    let mut files = Vec::new();
    for index in 0..30 {
        let path = project_root.join(format!("file{index}.ts"));
        let next = (index + 1) % 30;
        fs::write(
            &path,
            format!(
                "import {{ fn{next} }} from './file{next}';\nexport function fn{index}() {{ return fn{next}(); }}\n"
            ),
        )
        .expect("write measurement source");
        files.push(path);
    }

    let previous = std::env::var_os("AFT_CALLGRAPH_WRITE_AMP_BASELINE");
    if baseline {
        std::env::set_var("AFT_CALLGRAPH_WRITE_AMP_BASELINE", "1");
    } else {
        std::env::remove_var("AFT_CALLGRAPH_WRITE_AMP_BASELINE");
    }
    let (store, _) =
        CallGraphStore::cold_build_with_lease(store_dir.clone(), project_root.clone(), &files)
            .expect("cold-build measurement store");
    let mut previous_bytes = directory_bytes(&store_dir);
    let before_blocks = output_blocks();
    let mut directory_delta_bytes = 0;
    for edit in 0..50 {
        let path = &files[edit % files.len()];
        let mut source = fs::read_to_string(path).expect("read measurement source");
        source.push_str(&format!("// whitespace-preserving edit {edit}\n"));
        fs::write(path, source).expect("write measurement edit");
        store
            .refresh_files(std::slice::from_ref(path))
            .expect("refresh measurement edit");
        let current_bytes = directory_bytes(&store_dir);
        directory_delta_bytes += current_bytes.abs_diff(previous_bytes);
        previous_bytes = current_bytes;
    }
    drop(store);
    let measurement = WriteAmplificationMeasurement {
        directory_delta_bytes,
        output_blocks: output_blocks().saturating_sub(before_blocks),
    };
    match previous {
        Some(value) => std::env::set_var("AFT_CALLGRAPH_WRITE_AMP_BASELINE", value),
        None => std::env::remove_var("AFT_CALLGRAPH_WRITE_AMP_BASELINE"),
    }
    measurement
}

fn directory_bytes(path: &Path) -> u64 {
    let mut total = 0;
    for entry in fs::read_dir(path).expect("read measurement store") {
        let entry = entry.expect("read measurement entry");
        let metadata = entry.metadata().expect("stat measurement entry");
        total += if metadata.is_dir() {
            directory_bytes(&entry.path())
        } else {
            metadata.len()
        };
    }
    total
}

#[cfg(unix)]
fn output_blocks() -> u64 {
    unsafe {
        let mut usage = std::mem::zeroed();
        if libc::getrusage(libc::RUSAGE_SELF, &mut usage) == 0 {
            usage.ru_oublock as u64
        } else {
            0
        }
    }
}

#[cfg(not(unix))]
fn output_blocks() -> u64 {
    0
}
