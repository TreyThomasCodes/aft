//! Standing-root acceptance matrix.
//!
//! This integration target keeps the campaign's cross-surface contract visible
//! without duplicating the focused unit suites that own individual mechanisms.
//! The smoke tests below exercise the public lifecycle API across a shared
//! `aft.db`; the cited tests remain the exact executors for unit-shaped rows.
//!
//! | Acceptance row | Executor |
//! | --- | --- |
//! | Serialized entries, dependency closure, and trust boundary | `config_resolve::tests::index_roots_are_user_only_normalized_and_validate_before_resolution`; `config_resolve::tests::index_roots_project_and_mcp_tiers_are_rejected_at_the_trust_boundary` |
//! | Shared `aft.db`, migration, and busy-timeout-before-WAL | `db::tests::migration_runner_advances_version`; `db::tests::pragmas_applied_correctly`; `shared_database_and_overlapping_route_smoke` |
//! | Transactional strict clear and crash recovery | `db::standing_roots::tests::strict_verification_clear_is_atomic_and_drop_before_commit_keeps_flag`; `standing_roots::tests::crash_before_freshness_clear_blocks_verify_on_query_until_transactional_clear` |
//! | Resolved-path drift | `subc::health::tests::standing_health_names_resolved_path_drift_without_re_recording`; `scoped_key::tests::symlink_retargeting_resolves_to_a_different_pinned_identity` |
//! | Three artifact identities, scoped vectors, worktrees, and duplicates | `scoped_key::tests::subtree_key_never_equals_or_prewarms_the_repository_session_key`; `scoped_key::tests::non_git_roots_use_the_existing_path_scope_key`; `scoped_key::tests::scoped_v1_has_domain_separation_and_stable_bytes`; `scoped_key::tests::scoped_v1_preserves_unicode_case_and_never_normalizes`; `scoped_key::tests::scoped_v1_rejects_unsafe_logical_paths`; `scoped_key::tests::same_repo_worktree_and_logical_subtree_share_scoped_v1_key`; `scoped_key::tests::duplicate_artifact_keys_are_refused_before_admission` |
//! | Overlapping route selection and disclosure | `standing_roots::tests::route_falls_back_to_shallower_entry_and_discloses_it`; `shared_database_and_overlapping_route_smoke` |
//! | Initial and periodic maintenance, cadence, and coalescing | `subc::standing::tests::standing_interval_uses_the_existing_subc_maintenance_cadence`; `standing_roots::tests::maintenance_snapshot_preserves_configuration_and_fixed_kind_order`; executor coalescing is exercised by `subc::standing::StandingActor::submit_entry_pass` |
//! | Case-A publication fence and paired admission epoch | `standing_roots::tests::publication_is_a_noop_after_bind_epoch_revocation`; `standing_roots::tests::publication_fence_holds_epoch_mutex_through_final_rename`; `case_a_fence_and_resume_smoke` |
//! | Suspension-resume freshness and shared-key selection | `standing_roots::tests::suspension_edit_resume_requires_strict_verification_before_query`; `standing_roots::tests::shared_key_handoff_preserves_session_proven_kind_and_marks_other_kind`; `case_a_fence_and_resume_smoke` |
//! | Configuration snapshots and removals | `standing_roots::tests::configuration_add_modify_and_remove_mint_boundaries_and_delete_rows`; `standing_roots::tests::superseded_snapshot_publication_is_a_noop` |
//! | Standing limiter yielding at initial and checkpoint acquisition | `cold_build_limiter::tests::standing_yields_before_initial_and_checkpoint_reacquisition_when_non_standing_waits` |
//! | CLI no-op, trust, aggregation, disclosure, and shared freshness | `cli::index::tests::bare_empty_snapshot_is_an_explicit_successful_noop`; `cli::index::tests::validation_and_project_tier_trust_refusals_admit_no_builds`; `cli::index::tests::search_snapshot_discloses_zero_results_and_clears_shared_strict_state`; `cli::index::tests::aggregate_exit_states_preserve_usable_partial_output` |
//! | Stale-snapshot disclosure | **No executor exists in s1-s3:** `cli::index` builds snapshots but no standing query path emits the required stale disclosure naming `npx @cortexkit/aft index`. This is an inherited out-of-fence implementation gap, not covered by this slice. |
//! | Health, doctor, and memory | `subc::health::tests::standing_health_and_memory_reuse_existing_per_root_tables`; `subc::health::tests::standing_health_preserves_durable_breaker_reason`; doctor reads the same standing-written `callgraph/<artifact-key>/build-breaker.sqlite` directly through `build-breaker.ts::readBuildBreakerSuspensions`, then `doctor.ts::logBuildBreakerSuspensions` renders its stored `root_id`; `packages/aft-cli/src/lib/build-breaker.test.ts` (`renders the persisted domain, counter, age, reason, and reset command`) executes that path. |
//! | Network, budget, breaker, bind trust, and Windows | Existing shared paths remain unchanged; Windows scoped-key vectors run in `scoped_key::tests::windows_native_separators_match_logical_forward_slashes` on the Windows CI oracle. The local Parallels lane is not an oracle when its VM preflight is unavailable. |
//!
//! The s2 mutation-control rerun is `cargo test -p agent-file-tools --lib
//! standing_roots::tests --quiet`, which includes the epoch-fence, publication
//! mutex, durable strict-clear, crash, and suspension-resume regressions cited
//! above. This target deliberately exercises the same public transitions so a
//! later refactor cannot turn the map into documentation-only coverage.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use aft::config::{Config, IndexConfig, IndexKind, IndexRootConfig};
use aft::db::standing_roots::{get_standing_root, needs_strict_verify};
use aft::root_cache::{configure_artifact_access, RootCacheDomain, WriterLease};
use aft::standing_roots::{StandingRoots, StandingRouteError};

fn config(storage: &Path, roots: Vec<IndexRootConfig>) -> Config {
    Config {
        storage_dir: Some(storage.to_path_buf()),
        index: IndexConfig { roots },
        ..Config::default()
    }
}

fn root(path: &Path, indexes: Vec<IndexKind>) -> IndexRootConfig {
    IndexRootConfig {
        path: path.display().to_string(),
        indexes,
    }
}

#[test]
fn shared_database_and_overlapping_route_smoke() {
    let storage = tempfile::tempdir().expect("storage directory");
    let outer = tempfile::tempdir().expect("outer root");
    let inner = outer.path().join("inner");
    std::fs::create_dir(&inner).expect("inner root");
    let config = config(
        storage.path(),
        vec![
            root(outer.path(), vec![IndexKind::Search]),
            root(&inner, vec![IndexKind::Callgraph]),
        ],
    );

    let daemon = StandingRoots::default();
    daemon.reconcile(&config).expect("daemon reconciliation");
    let literal_outer = outer.path().display().to_string();
    let conn = aft::db::open(&storage.path().join("aft.db")).expect("shared database");
    assert!(get_standing_root(&conn, &literal_outer)
        .expect("read daemon record")
        .is_some());
    drop(conn);

    // A separate lifecycle owner observes the same durable entry, as a
    // daemonless CLI snapshot does before it clears a strict-verification row.
    let cli = StandingRoots::default();
    cli.reconcile(&config).expect("CLI reconciliation");
    cli.record_strict_verification(&literal_outer, IndexKind::Search)
        .expect("CLI strict verification");
    let conn = aft::db::open(&storage.path().join("aft.db")).expect("shared database");
    assert_eq!(
        needs_strict_verify(&conn, &literal_outer, IndexKind::Search).expect("freshness row"),
        Some(false)
    );

    let route = cli
        .route_explicit_path(&inner.join("query.rs"), IndexKind::Search)
        .expect("shallower selected entry serves search");
    assert_eq!(route.entry.literal_path, literal_outer);
    assert_eq!(
        cli.route_explicit_path(&inner.join("query.rs"), IndexKind::Semantic),
        Err(StandingRouteError::KindUnavailable {
            deepest_literal_path: inner.display().to_string(),
            deepest_selection: vec![IndexKind::Callgraph],
        })
    );
}

#[test]
fn case_a_fence_and_resume_smoke() {
    let storage = tempfile::tempdir().expect("storage directory");
    let root_dir = tempfile::tempdir().expect("standing root");
    let literal_path = root_dir.path().display().to_string();
    let roots = StandingRoots::default();
    let report = roots
        .reconcile(&config(
            storage.path(),
            vec![root(root_dir.path(), vec![IndexKind::Search])],
        ))
        .expect("standing reconciliation");
    let entry = report
        .active_entries
        .into_iter()
        .next()
        .expect("standing entry");
    let admission = roots
        .admit_build(&literal_path)
        .expect("standing admission");
    configure_artifact_access(&entry.resolved_target, &entry.artifact_key, false);
    let cache_dir = storage.path().join("index").join(&entry.artifact_key);
    let lease = WriterLease::acquire_shared(
        RootCacheDomain::Index,
        &cache_dir,
        &entry.artifact_key,
        &entry.resolved_target,
    )
    .expect("writer lease result")
    .expect("writer lease");

    roots.begin_case_a_bind(&literal_path).expect("Case-A bind");
    let published = AtomicBool::new(false);
    assert!(roots
        .publish_if_current(
            &literal_path,
            admission.publication,
            &lease,
            || true,
            || true,
            || published.store(true, Ordering::SeqCst),
        )
        .expect("publication fence")
        .is_none());
    assert!(!published.load(Ordering::SeqCst));
    assert!(admission.checkpoint());

    roots
        .resume_after_session(&literal_path, &[])
        .expect("standing resume");
    assert!(matches!(
        roots.route_explicit_path(&root_dir.path().join("query.rs"), IndexKind::Search),
        Err(StandingRouteError::StrictVerificationRequired { .. })
    ));
}
