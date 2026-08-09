use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, LazyLock};
use std::thread;
use std::time::Instant;

use crossbeam_channel::{unbounded, Receiver, Sender};

use super::job::{InspectCategory, InspectJob, InspectResult};

pub type InspectWorker = Arc<dyn Fn(InspectJob) -> InspectResult + Send + Sync + 'static>;

#[derive(Clone)]
pub struct DispatchHandles {
    pub request_tx: Sender<InspectJob>,
    pub result_rx: Receiver<InspectResult>,
    pub pool: Arc<rayon::ThreadPool>,
}

/// Number of live workers in the process-wide inspect pool. The pool's start
/// and exit handlers maintain this counter for platform-independent diagnostics.
static INSPECT_THREAD_COUNT: AtomicUsize = AtomicUsize::new(0);

/// The inspect pool is process-wide because roots are independently evictable
/// while their scans still share the daemon process. Inspect mixes file IO and
/// parsing; `min(8, available_parallelism)` is the measured point where more
/// workers stop improving the supported repositories while leaving cores for
/// interactive lanes.
///
/// `ColdBuildLimiter` remains separate: it admits concurrent heavy operations
/// (including Tier-2 scans), while this pool bounds the threads those admitted
/// operations can use. The pool is intentionally process-lifetime; dropping a
/// root's `InspectManager` only releases that root's handle to this pool.
static INSPECT_POOL: LazyLock<Arc<rayon::ThreadPool>> = LazyLock::new(|| {
    Arc::new(
        rayon::ThreadPoolBuilder::new()
            .num_threads(default_pool_size())
            .thread_name(|index| format!("aft-inspect-{index}"))
            // Rayon defaults workers to ~2MB stacks (vs the main thread's 8MB).
            // The duplicates scanner walks the AST recursively, and deep trees
            // (minified bundles, generated code, long chains) previously
            // overflowed a 2MB worker stack and SIGABRT'd the whole bridge.
            // Match the main thread's 8MB so the bounded recursion in
            // collect_fragments (MAX_FRAGMENT_DEPTH) has comfortable headroom.
            .stack_size(8 * 1024 * 1024)
            .start_handler(|_| {
                INSPECT_THREAD_COUNT.fetch_add(1, Ordering::SeqCst);
            })
            .exit_handler(|_| {
                INSPECT_THREAD_COUNT.fetch_sub(1, Ordering::SeqCst);
            })
            .build()
            .expect("inspect worker pool must build"),
    )
});

pub fn start_dispatch_loop(worker: InspectWorker) -> DispatchHandles {
    let (request_tx, request_rx) = unbounded::<InspectJob>();
    let (result_tx, result_rx) = unbounded::<InspectResult>();
    let pool = inspect_pool();

    let loop_pool = Arc::clone(&pool);
    thread::spawn(move || dispatch_loop(request_rx, result_tx, loop_pool, worker));

    DispatchHandles {
        request_tx,
        result_rx,
        pool,
    }
}

fn inspect_pool() -> Arc<rayon::ThreadPool> {
    Arc::clone(&INSPECT_POOL)
}

#[doc(hidden)]
pub fn inspect_pool_size_for_test() -> usize {
    inspect_pool().current_num_threads()
}

#[doc(hidden)]
pub fn inspect_pool_thread_count_for_test() -> usize {
    INSPECT_THREAD_COUNT.load(Ordering::SeqCst)
}

pub fn default_worker() -> InspectWorker {
    Arc::new(dispatch_category)
}

fn dispatch_loop(
    request_rx: Receiver<InspectJob>,
    result_tx: Sender<InspectResult>,
    pool: Arc<rayon::ThreadPool>,
    worker: InspectWorker,
) {
    while let Ok(job) = request_rx.recv() {
        let tx = result_tx.clone();
        let worker = Arc::clone(&worker);
        pool.spawn_fifo(move || {
            let result = worker(job);
            let _ = tx.send(result);
        });
    }
}

fn dispatch_category(job: InspectJob) -> InspectResult {
    use crate::inspect::scanners;

    match job.category {
        InspectCategory::Todos => scanners::todos::run_todos_scan(&job),
        InspectCategory::Metrics => scanners::metrics::run_metrics_scan(&job),
        InspectCategory::DeadCode => scanners::dead_code::run_dead_code_scan(&job),
        InspectCategory::UnusedExports => scanners::unused_exports::run_unused_exports_scan(&job),
        InspectCategory::Duplicates => scanners::duplicates::run_duplicates_scan(&job),
        InspectCategory::Cycles => scanners::cycles::run_cycles_scan(&job),
        InspectCategory::Diagnostics => {
            // Diagnostics are backed by the AppContext LSP manager and run via
            // the serial LSP/status lane in `handle_inspect` — never through
            // this rayon worker pool. Reaching this arm means a caller routed
            // Diagnostics into the worker path incorrectly; surface that as a
            // routing bug instead of a misleading "pending" status.
            let started = Instant::now();
            InspectResult::failed(
                &job,
                "diagnostics must run on the main thread (run_diagnostics_category), \
                 not the rayon inspect worker pool",
                started.elapsed(),
            )
        }
        other => {
            let started = Instant::now();
            InspectResult::failed(
                &job,
                format!("inspect category '{other}' is not active in v0.33"),
                started.elapsed(),
            )
        }
    }
}

fn default_pool_size() -> usize {
    std::thread::available_parallelism()
        .map(|parallelism| parallelism.get())
        .unwrap_or(1)
        .min(8)
}
