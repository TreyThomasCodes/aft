use crate::db::github_read_cache::{
    lookup_github_read_cache_entry, upsert_github_read_cache_entry, GithubReadCacheKey,
};
use rusqlite::Connection;

fn github_read_mutation_request(
    action: &str,
    repository: &str,
    resource_number: i64,
) -> GovernedRequest {
    let mut target = serde_json::Map::new();
    target.insert(
        "number".to_string(),
        serde_json::Value::String(resource_number.to_string()),
    );
    GovernedRequest {
        action: action.to_string(),
        target,
        body: serde_json::Map::new(),
        repository: Some(repository.to_string()),
        manifest_version: 1,
        edit_last: false,
    }
}

fn cache_key(repository: &str, resource_number: i64, identity: &str) -> GithubReadCacheKey {
    GithubReadCacheKey::new(
        GithubReadResourceKind::Issue,
        repository,
        resource_number,
        identity,
    )
}

fn write_cached_issue(
    conn: &Connection,
    repository: &str,
    resource_number: i64,
    identity: &str,
) {
    upsert_github_read_cache_entry(
        conn,
        &cache_key(repository, resource_number, identity),
        "# Cached issue\n",
        1_000,
    )
    .expect("write cached issue");
}

fn cached_issue_exists(
    conn: &Connection,
    repository: &str,
    resource_number: i64,
    identity: &str,
) -> bool {
    lookup_github_read_cache_entry(conn, &cache_key(repository, resource_number, identity))
        .expect("look up cached issue")
        .is_some()
}

#[test]
fn successful_structured_comment_mutation_invalidates_the_touched_issue_for_all_identities() {
    let storage = tempfile::tempdir().expect("create storage");
    let conn = crate::db::open(&storage.path().join("aft.db")).expect("open cache database");
    write_cached_issue(&conn, "cortexkit/aft", 42, "principal:alice");
    write_cached_issue(&conn, "cortexkit/aft", 42, "principal:bob");

    let request = github_read_mutation_request("issue comment", "CortexKit/AFT", 42);
    let mutation = GithubReadMutation::from_governed_request(&request)
        .expect("structured issue comment has a cache resource");
    assert_eq!(mutation.normalized_repository, "cortexkit/aft");
    assert_eq!(mutation.resource_kind, GithubReadResourceKind::Issue);
    assert_eq!(mutation.resource_number, 42);

    invalidate_successful_github_read_mutation_at(
        storage.path(),
        Some(&mutation),
        &RouteOutcome::Result("comment created".to_string()),
    );

    assert!(
        !cached_issue_exists(&conn, "cortexkit/aft", 42, "principal:alice"),
        "a successful comment invalidates Alice's cached issue"
    );
    assert!(
        !cached_issue_exists(&conn, "cortexkit/aft", 42, "principal:bob"),
        "a successful comment invalidates every identity's cached issue"
    );
}

#[test]
fn failed_structured_comment_mutation_does_not_invalidate_the_touched_issue() {
    let storage = tempfile::tempdir().expect("create storage");
    let conn = crate::db::open(&storage.path().join("aft.db")).expect("open cache database");
    write_cached_issue(&conn, "cortexkit/aft", 42, "principal:alice");

    let request = github_read_mutation_request("issue comment", "cortexkit/aft", 42);
    let mutation = GithubReadMutation::from_governed_request(&request)
        .expect("structured issue comment has a cache resource");
    invalidate_successful_github_read_mutation_at(
        storage.path(),
        Some(&mutation),
        &RouteOutcome::UpstreamError("comment rejected".to_string()),
    );

    assert!(
        cached_issue_exists(&conn, "cortexkit/aft", 42, "principal:alice"),
        "a failed mutation must preserve the cached issue"
    );
}

#[test]
fn successful_mutation_for_a_different_issue_leaves_the_control_entry_intact() {
    let storage = tempfile::tempdir().expect("create storage");
    let conn = crate::db::open(&storage.path().join("aft.db")).expect("open cache database");
    write_cached_issue(&conn, "cortexkit/aft", 42, "principal:alice");
    write_cached_issue(&conn, "cortexkit/aft", 43, "principal:alice");

    let request = github_read_mutation_request("issue comment", "cortexkit/aft", 43);
    let mutation = GithubReadMutation::from_governed_request(&request)
        .expect("structured issue comment has a cache resource");
    invalidate_successful_github_read_mutation_at(
        storage.path(),
        Some(&mutation),
        &RouteOutcome::Result("comment created".to_string()),
    );

    assert!(
        cached_issue_exists(&conn, "cortexkit/aft", 42, "principal:alice"),
        "a mutation for another issue must not evict the control entry"
    );
    assert!(
        !cached_issue_exists(&conn, "cortexkit/aft", 43, "principal:alice"),
        "the successful mutation must still evict its own issue"
    );
}
