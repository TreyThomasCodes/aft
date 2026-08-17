//! Credential-free routing shim for the `gh` argv[0] entry point.
//!
//! The shim is intentionally a small process boundary: R1/R2 and declared
//! mechanical R3 operations replace this process with upstream `gh`, while the
//! governed path is the only path that interprets a declared command shape.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use ring::signature::{UnparsedPublicKey, ED25519};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use subc_client_rs::{CallOptions, CloseRouteOptions, ConsumerOptions, SubcConsumer};
use subc_protocol::manifest::ProviderRole;
use subc_protocol::{BindIdentity, RouteTarget};

pub const SCHEMA_FLOOR: u64 = 1;
pub const REFUSAL_EXIT_STATUS: i32 = 86;
const DISCOVERY_BUDGET: Duration = Duration::from_millis(150);
const DISCOVERY_CACHE_TTL: Duration = Duration::from_secs(15);
const MANIFEST_TTL: Duration = Duration::from_secs(15 * 60);
const MANIFEST_STALE_GRACE: Duration = Duration::from_secs(24 * 60 * 60);
const ROUTING_OPERATION: &str = "gh.route";
const ROUTING_HOLDER_MODULE_ID: &str = "prefrontal-core";
const MANIFEST_ARTIFACT_ID: &str = "gh-routing-manifest";
const RESERVED_SELF_REPORT: &[&str] = &["--status", "--shim-version"];

/// The only shim-originated refusal identifiers. Keep this enumeration closed:
/// callers must parse these identifiers rather than human prose.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RefusalCode {
    Unclassified,
    AdminTier,
    ManifestStale,
    ManifestBelowFloor,
    SeamSchemaMismatch,
    UnboundIdentity,
    BypassAuditUnavailable,
    NoRealGh,
    SeamUnavailable,
    SeamRefusal,
}

impl RefusalCode {
    pub const ALL: [Self; 10] = [
        Self::Unclassified,
        Self::AdminTier,
        Self::ManifestStale,
        Self::ManifestBelowFloor,
        Self::SeamSchemaMismatch,
        Self::UnboundIdentity,
        Self::BypassAuditUnavailable,
        Self::NoRealGh,
        Self::SeamUnavailable,
        Self::SeamRefusal,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unclassified => "gh_shim_unclassified",
            Self::AdminTier => "gh_shim_admin_tier",
            Self::ManifestStale => "gh_shim_manifest_stale",
            Self::ManifestBelowFloor => "gh_shim_manifest_below_floor",
            Self::SeamSchemaMismatch => "gh_shim_seam_schema_mismatch",
            Self::UnboundIdentity => "gh_shim_unbound_identity",
            Self::BypassAuditUnavailable => "gh_shim_bypass_audit_unavailable",
            Self::NoRealGh => "gh_shim_no_real_gh",
            Self::SeamUnavailable => "gh_shim_seam_unavailable",
            Self::SeamRefusal => "gh_shim_seam_refusal",
        }
    }
}

/// Offline self-report uses diagnostic identifiers distinct from invocation
/// refusals. A report can therefore describe historical local-state trouble
/// without pretending that an upstream `gh` invocation was refused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SelfReportDiagnostic {
    ManifestUnavailable,
    ManifestInvalid,
    ManifestBelowFloor,
    ManifestStale,
    RungUnavailable,
}

impl SelfReportDiagnostic {
    pub const ALL: [Self; 5] = [
        Self::ManifestUnavailable,
        Self::ManifestInvalid,
        Self::ManifestBelowFloor,
        Self::ManifestStale,
        Self::RungUnavailable,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ManifestUnavailable => "gh_shim_status_manifest_unavailable",
            Self::ManifestInvalid => "gh_shim_status_manifest_invalid",
            Self::ManifestBelowFloor => "gh_shim_status_manifest_below_floor",
            Self::ManifestStale => "gh_shim_status_manifest_stale",
            Self::RungUnavailable => "gh_shim_status_rung_unavailable",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Tier {
    Mechanical,
    Governed,
    Admin,
}

impl Tier {
    fn rank(self) -> u8 {
        match self {
            Self::Mechanical => 0,
            Self::Governed => 1,
            Self::Admin => 2,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Rung {
    R1,
    R2,
    R3,
}

impl Rung {
    const fn label(self) -> &'static str {
        match self {
            Self::R1 => "R1",
            Self::R2 => "R2",
            Self::R3 => "R3",
        }
    }
}

/// Return true when the process was invoked through the `gh` symlink or the
/// explicit `aft gh-shim` development entry point. This is public so the binary
/// can perform it before its own global `--version` and `--subc` scans.
pub fn is_shim_invocation(program: &OsStr, args: &[OsString]) -> bool {
    Path::new(program)
        .file_name()
        .is_some_and(|name| name == OsStr::new("gh"))
        || args.first().is_some_and(|arg| arg == OsStr::new("gh-shim"))
}

pub fn is_shim_invocation_from_env() -> bool {
    let mut argv = std::env::args_os();
    let Some(program) = argv.next() else {
        return false;
    };
    is_shim_invocation(&program, &argv.collect::<Vec<_>>())
}

/// Execute the shim for either supported entry form. This intentionally runs
/// before logging initialization so delegating invocations cannot add shim bytes
/// to upstream stderr.
pub fn run_from_env() -> i32 {
    let mut argv = std::env::args_os();
    let Some(program) = argv.next() else {
        return refuse(RefusalCode::NoRealGh, "the executing image was unavailable");
    };
    let raw_args = argv.collect::<Vec<_>>();
    let shim_args = if Path::new(&program)
        .file_name()
        .is_some_and(|name| name == OsStr::new("gh"))
    {
        raw_args
    } else {
        raw_args.into_iter().skip(1).collect()
    };
    run(&shim_args)
}

fn run(args: &[OsString]) -> i32 {
    let paths = StatePaths::from_process();
    if is_reserved_self_report(args) {
        print_self_report(&paths);
        return 0;
    }

    let now = unix_seconds();
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let determination = determine_rung(&paths, &cwd, now);
    if determination.rung != Rung::R3 {
        return delegate(args);
    }

    // A valid manifest gates R3 both during fresh discovery and when a cached
    // R3 determination is reused. If it disappears or expires between those
    // two moments, B1 requires whole-invocation R2 passthrough instead of a
    // classification-shaped refusal.
    let manifest = match load_manifest(&paths, now) {
        Ok(manifest) => manifest,
        Err(_) => return delegate(args),
    };

    match classify(args, &manifest, current_platform()) {
        Classification::Mechanical => delegate(args),
        Classification::Admin { tuple } => {
            if std::env::var_os("GH_SHIM_BYPASS").as_deref() == Some(OsStr::new("operator")) {
                let repository = explicit_repo(args).or_else(infer_repository_from_git);
                if let Err(error) = append_bypass_audit(&paths, &tuple, repository.as_deref(), now)
                {
                    return refuse(
                        RefusalCode::BypassAuditUnavailable,
                        &format!("operator bypass audit could not be appended: {error}"),
                    );
                }
                delegate(args)
            } else {
                refuse(
                    RefusalCode::AdminTier,
                    "this action requires GH_SHIM_BYPASS=operator",
                )
            }
        }
        Classification::Governed { tuple, canonical } => {
            let request =
                match canonicalize_governed(args, &tuple, &canonical, manifest.manifest_version) {
                    Ok(request) => request,
                    Err(_) => {
                        return refuse(
                            RefusalCode::Unclassified,
                            &format!(
                                "undeclared shape for {tuple} in manifest {}",
                                manifest.manifest_version
                            ),
                        )
                    }
                };
            match route_governed(&paths, &determination, request) {
                RouteOutcome::Result(output) => {
                    print!("{output}");
                    0
                }
                RouteOutcome::Refusal(code) => refuse(
                    RefusalCode::SeamRefusal,
                    &format!("governance seam refused the action: {code}"),
                ),
                RouteOutcome::UnboundIdentity => refuse(
                    RefusalCode::UnboundIdentity,
                    "the project binding was unavailable at route time",
                ),
                RouteOutcome::SchemaMismatch(message) => {
                    refuse(RefusalCode::SeamSchemaMismatch, &message)
                }
                RouteOutcome::Unavailable(message) => {
                    refuse(RefusalCode::SeamUnavailable, &message)
                }
            }
        }
        Classification::Unclassified => refuse(
            RefusalCode::Unclassified,
            &format!(
                "no manifest declaration for this invocation (manifest {})",
                manifest.manifest_version
            ),
        ),
    }
}

fn is_reserved_self_report(args: &[OsString]) -> bool {
    args.first()
        .and_then(|arg| arg.to_str())
        .is_some_and(|arg| RESERVED_SELF_REPORT.contains(&arg))
}

#[derive(Clone, Debug)]
struct StatePaths {
    root: PathBuf,
    manifest: PathBuf,
    rung: PathBuf,
    bypass_audit: PathBuf,
    unexpected_gh_route_advertisers: PathBuf,
}

impl StatePaths {
    fn from_process() -> Self {
        let root = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
            .or_else(|| {
                std::env::var_os("HOME")
                    .or_else(|| std::env::var_os("USERPROFILE"))
                    .map(|home| PathBuf::from(home).join(".local/state"))
            })
            .unwrap_or_else(|| std::env::temp_dir())
            .join("cortexkit")
            .join("aft")
            .join("gh-shim");
        Self::from_root(root)
    }

    fn from_root(root: PathBuf) -> Self {
        Self {
            manifest: root.join("gh-routing-manifest.json"),
            rung: root.join("rung-cache.json"),
            bypass_audit: root.join("operator-bypass.jsonl"),
            unexpected_gh_route_advertisers: root.join("unexpected-gh-route-advertisers.json"),
            root,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct RungRecord {
    rung: Rung,
    as_of_unix_secs: u64,
    #[serde(default)]
    inputs: BTreeMap<String, String>,
    #[serde(default)]
    manifest_version: Option<u64>,
}

impl RungRecord {
    fn r1(now: u64, reason: &str) -> Self {
        Self {
            rung: Rung::R1,
            as_of_unix_secs: now,
            inputs: BTreeMap::from([("connection_file".to_string(), reason.to_string())]),
            manifest_version: None,
        }
    }

    fn r2(now: u64, reason: &str, manifest_version: Option<u64>) -> Self {
        Self {
            rung: Rung::R2,
            as_of_unix_secs: now,
            inputs: BTreeMap::from([
                ("connection_file".to_string(), "ready".to_string()),
                (reason.to_string(), "failed".to_string()),
            ]),
            manifest_version,
        }
    }

    fn r3(now: u64, manifest_version: u64) -> Self {
        Self {
            rung: Rung::R3,
            as_of_unix_secs: now,
            inputs: BTreeMap::from([
                ("connection_file".to_string(), "ready".to_string()),
                ("catalog_gh_route".to_string(), "ready".to_string()),
                ("agent_binding".to_string(), "ready".to_string()),
                ("manifest".to_string(), "ready".to_string()),
                (
                    "agent_credentials_present".to_string(),
                    "absent".to_string(),
                ),
            ]),
            manifest_version: Some(manifest_version),
        }
    }

    fn fresh_at(&self, now: u64) -> bool {
        now.saturating_sub(self.as_of_unix_secs) < DISCOVERY_CACHE_TTL.as_secs()
    }
}

fn determine_rung(paths: &StatePaths, cwd: &Path, now: u64) -> RungRecord {
    // The budget starts before the config read and connection-file stat. This
    // keeps a slow filesystem from silently extending discovery beyond 150ms.
    let deadline = std::time::Instant::now() + DISCOVERY_BUDGET;
    let Some(connection_file) = configured_connection_file() else {
        // R1 has no daemon dial and no durable determination write.
        return RungRecord::r1(now, "absent_or_unparseable");
    };

    let cached = load_rung_record(paths);
    if std::time::Instant::now() >= deadline {
        return cached
            .filter(|record| record.fresh_at(now))
            .unwrap_or_else(|| RungRecord::r1(now, "discovery_budget_exhausted"));
    }
    if let Some(record) = cached.as_ref().filter(|record| record.fresh_at(now)) {
        if record.rung != Rung::R3 || load_manifest(paths, now).is_ok() {
            return record.clone();
        }
    }

    let discovery = probe_governance(paths, &connection_file, cwd, deadline);
    let record = match discovery {
        ProbeResult::Ready { module_id } => {
            // The manifest is checked before the detector because it defines the
            // complete detector inventory. An invalid cache therefore cannot be
            // used to select a detector or activate R3.
            match load_manifest(paths, now) {
                Ok(manifest) => match find_ambient_agent_credential(&manifest.detectors) {
                    Some(source) => {
                        let mut record = RungRecord::r2(
                            now,
                            "agent_credentials_present",
                            Some(manifest.manifest_version),
                        );
                        record
                            .inputs
                            .insert("agent_credentials_present".to_string(), source);
                        record
                            .inputs
                            .insert("catalog_holder".to_string(), module_id);
                        record
                    }
                    None => RungRecord::r3(now, manifest.manifest_version),
                },
                Err(_) => {
                    let mut record = RungRecord::r2(now, "manifest_unavailable", None);
                    record
                        .inputs
                        .insert("catalog_gh_route".to_string(), "ready".to_string());
                    record
                        .inputs
                        .insert("agent_binding".to_string(), "ready".to_string());
                    record
                }
            }
        }
        ProbeResult::Unreachable => RungRecord::r2(now, "daemon_unreachable", None),
        ProbeResult::NoRoute => RungRecord::r2(now, "catalog_gh_route_absent", None),
        ProbeResult::Unbound => RungRecord::r2(now, "agent_binding_unavailable", None),
        ProbeResult::TimedOut => cached
            .filter(|record| record.fresh_at(now))
            .unwrap_or_else(|| RungRecord::r1(now, "discovery_budget_exhausted")),
    };

    if record.rung != Rung::R1 {
        write_rung_record_silently(paths, &record);
    }
    record
}

fn configured_connection_file() -> Option<PathBuf> {
    let xdg_config_home = std::env::var_os("XDG_CONFIG_HOME");
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
    configured_connection_file_from(xdg_config_home.as_deref(), home.as_deref())
}

fn configured_connection_file_from(
    xdg_config_home: Option<&OsStr>,
    home: Option<&OsStr>,
) -> Option<PathBuf> {
    // The shim uses the same user-tier resolver as subc: `$XDG_CONFIG_HOME/cortexkit/aft.jsonc`,
    // then `~/.config/cortexkit/aft.jsonc`. XDG selects only the trusted user's
    // config location; it cannot select a project file or alter the configured
    // connection. If that path is invalid, resolution falls back to the lower-priority
    // R1 route instead of allowing a routing bypass.
    let config_path = crate::subc_config::user_config_path_from(xdg_config_home, home)?;
    let doc = fs::read_to_string(config_path).ok()?;
    connection_file_from_config_doc(&doc).filter(|path| path.is_file())
}

fn connection_file_from_config_doc(doc: &str) -> Option<PathBuf> {
    let value: Value = serde_json::from_str(&crate::jsonc::strip_jsonc(doc)).ok()?;
    let raw = value.get("subc")?.get("connection_file")?.as_str()?.trim();
    let path = PathBuf::from(raw);
    (!raw.is_empty() && path.is_absolute()).then_some(path)
}

fn load_rung_record(paths: &StatePaths) -> Option<RungRecord> {
    serde_json::from_slice(&fs::read(&paths.rung).ok()?).ok()
}

fn write_rung_record_silently(paths: &StatePaths, record: &RungRecord) {
    let Ok(bytes) = serde_json::to_vec(record) else {
        return;
    };
    let _ = fs::create_dir_all(&paths.root);
    let temporary = paths.root.join("rung-cache.json.tmp");
    if fs::write(&temporary, bytes).is_ok() {
        let _ = fs::rename(temporary, &paths.rung);
    }
}

#[derive(Debug)]
enum ProbeResult {
    Ready { module_id: String },
    Unreachable,
    NoRoute,
    Unbound,
    TimedOut,
}

fn probe_governance(
    paths: &StatePaths,
    connection_file: &Path,
    cwd: &Path,
    deadline: std::time::Instant,
) -> ProbeResult {
    let remaining = deadline.saturating_duration_since(std::time::Instant::now());
    if remaining.is_zero() {
        return ProbeResult::TimedOut;
    }
    let connection_file = connection_file.to_path_buf();
    let project_root = project_root_for(cwd);
    let record_paths = paths.clone();
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
    else {
        return ProbeResult::Unreachable;
    };

    match runtime.block_on(tokio::time::timeout(remaining, async move {
        let options = ConsumerOptions {
            call_timeout: remaining,
            ..ConsumerOptions::default()
        };
        let consumer = SubcConsumer::connect(&connection_file, options)
            .await
            .map_err(|_| ProbeResult::Unreachable)?;
        let catalog = consumer
            .catalog_list()
            .await
            .map_err(|_| ProbeResult::Unreachable)?;
        let holder = route_holder(&catalog.modules);
        record_unexpected_gh_route_advertisers(&record_paths, &holder.unexpected_advertisers);
        let Some(module_id) = holder.module_id else {
            return Err(ProbeResult::NoRoute);
        };
        let identity = BindIdentity {
            project_root: project_root.to_string_lossy().into_owned().into(),
            harness: "aft-gh-shim".to_string(),
            session: format!("gh-shim-{}", std::process::id()),
        };
        let route = consumer
            .open_route(
                RouteTarget::ManagementSurface {
                    module_id: module_id.clone(),
                },
                identity,
                CallOptions::default(),
            )
            .await
            .map_err(|_| ProbeResult::Unbound)?;
        let _ = consumer
            .close_handle(&route, CloseRouteOptions::default())
            .await;
        Ok(module_id)
    })) {
        Ok(Ok(module_id)) => ProbeResult::Ready { module_id },
        Ok(Err(result)) => result,
        Err(_) => ProbeResult::TimedOut,
    }
}

#[derive(Debug, Default, Eq, PartialEq)]
struct RouteHolder {
    module_id: Option<String>,
    unexpected_advertisers: Vec<String>,
}

fn route_holder(entries: &[subc_client_rs::CatalogEntry]) -> RouteHolder {
    select_route_holder(entries.iter().filter_map(|entry| {
        entry
            .roles
            .iter()
            .any(|role| {
                matches!(
                    role,
                    ProviderRole::ManagementSurface { operations, .. }
                        if operations.iter().any(|operation| operation.name == ROUTING_OPERATION)
                )
            })
            .then(|| entry.module_id.clone())
    }))
}

fn select_route_holder(advertisers: impl IntoIterator<Item = String>) -> RouteHolder {
    let mut holder = None;
    let mut unexpected_advertisers = BTreeSet::new();
    for advertiser in advertisers {
        // Governed routes carry identity-bearing writes, so only prefrontal-core may
        // hold `gh.route`; another module advertising it must not capture the route.
        if advertiser == ROUTING_HOLDER_MODULE_ID {
            holder.get_or_insert(advertiser);
        } else {
            unexpected_advertisers.insert(advertiser);
        }
    }
    RouteHolder {
        module_id: holder,
        unexpected_advertisers: unexpected_advertisers.into_iter().collect(),
    }
}

fn project_root_for(cwd: &Path) -> PathBuf {
    let canonical = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    canonical
        .ancestors()
        .find(|path| path.join(".git").exists())
        .map(Path::to_path_buf)
        .unwrap_or(canonical)
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct Detectors {
    #[serde(default)]
    wrapper_config_dirs: Vec<String>,
    #[serde(default)]
    credential_env_names: Vec<String>,
}

fn find_ambient_agent_credential(detectors: &Detectors) -> Option<String> {
    for name in &detectors.credential_env_names {
        if std::env::var_os(name).is_some() {
            return Some(format!("env:{name}"));
        }
    }

    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    for raw_pattern in &detectors.wrapper_config_dirs {
        let pattern = expand_home_pattern(raw_pattern, home.as_deref());
        if let Ok(paths) = glob::glob(&pattern) {
            for path in paths.flatten() {
                if path.is_dir() {
                    return Some(format!("path:{}", path.display()));
                }
            }
        }
    }

    // `GH_CONFIG_DIR` is only inspected as a metadata path. The basename is
    // compared to the manifest's declared wrapper-dir glob, so the operator's
    // normal gh configuration remains outside this detector inventory.
    let configured = std::env::var_os("GH_CONFIG_DIR").map(PathBuf::from)?;
    if !configured.is_dir() {
        return None;
    }
    let name = configured.file_name()?.to_string_lossy();
    detectors
        .wrapper_config_dirs
        .iter()
        .any(|pattern| {
            Path::new(pattern).file_name().is_some_and(|glob_name| {
                glob::Pattern::new(&glob_name.to_string_lossy()).is_ok_and(|p| p.matches(&name))
            })
        })
        .then(|| format!("path:{}", configured.display()))
}

fn expand_home_pattern(pattern: &str, home: Option<&Path>) -> String {
    pattern
        .strip_prefix("~/")
        .and_then(|suffix| home.map(|home| home.join(suffix).to_string_lossy().into_owned()))
        .unwrap_or_else(|| pattern.to_string())
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
enum TupleDecl {
    Name(String),
    Details {
        tuple: String,
        #[serde(default)]
        platform: Vec<String>,
        #[serde(default)]
        api_match: Option<String>,
        #[serde(default)]
        rationale: Option<String>,
    },
}

impl TupleDecl {
    fn tuple(&self) -> &str {
        match self {
            Self::Name(name) => name,
            Self::Details { tuple, .. } => tuple,
        }
    }

    fn platform(&self) -> &[String] {
        match self {
            Self::Name(_) => &[],
            Self::Details { platform, .. } => platform,
        }
    }

    fn empty_api_match_has_rationale(&self) -> bool {
        match self {
            Self::Details {
                api_match: Some(api_match),
                rationale,
                ..
            } if api_match.is_empty() => rationale
                .as_deref()
                .is_some_and(|text| !text.trim().is_empty()),
            _ => true,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct ApiRule {
    method: String,
    path_glob: String,
    tier: Tier,
    #[serde(default)]
    platform: Vec<String>,
    #[serde(default)]
    rationale: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct Canonicalization {
    #[serde(default)]
    argv_forms: Vec<String>,
    #[serde(default)]
    target_fields: Vec<String>,
    #[serde(default)]
    body_fields: Vec<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct RepositorySection {
    #[serde(default)]
    tiers: BTreeMap<Tier, Vec<TupleDecl>>,
    #[serde(default, alias = "remove")]
    removed_tuples: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Manifest {
    artifact_id: String,
    manifest_version: u64,
    schema_floor: u64,
    #[serde(default)]
    detectors: Detectors,
    #[serde(default)]
    tiers: BTreeMap<Tier, Vec<TupleDecl>>,
    #[serde(default)]
    api_rules: Vec<ApiRule>,
    #[serde(default)]
    canonicalization: BTreeMap<String, Canonicalization>,
    #[serde(default)]
    repository_sections: BTreeMap<String, RepositorySection>,
}

impl Manifest {
    fn validate(&self) -> Result<(), String> {
        if self.artifact_id != MANIFEST_ARTIFACT_ID {
            return Err(format!("unexpected artifact id {}", self.artifact_id));
        }
        if self.manifest_version == 0 {
            return Err("manifest_version must be positive".to_string());
        }

        let mut declared = BTreeMap::<String, Tier>::new();
        for (tier, entries) in &self.tiers {
            for entry in entries {
                let tuple = normalized_tuple(entry.tuple())?;
                if entry.platform().is_empty() {
                    return Err(format!("tuple {tuple} is missing its platform declaration"));
                }
                if !entry.empty_api_match_has_rationale() {
                    return Err(format!(
                        "tuple {tuple} has an empty api_match without rationale"
                    ));
                }
                if let Some(previous) = declared.insert(tuple.clone(), *tier) {
                    return Err(format!(
                        "tuple {tuple} is declared in both {previous:?} and {tier:?}"
                    ));
                }
            }
        }

        let mut api_declared = BTreeSet::new();
        for rule in &self.api_rules {
            if rule.method.trim().is_empty() || rule.path_glob.trim().is_empty() {
                if rule.path_glob.is_empty()
                    && rule
                        .rationale
                        .as_deref()
                        .is_some_and(|text| !text.trim().is_empty())
                {
                    continue;
                }
                return Err("api rule requires method and non-empty path_glob".to_string());
            }
            if rule.platform.is_empty() {
                return Err(format!(
                    "api rule {} {} is missing its platform declaration",
                    rule.method, rule.path_glob
                ));
            }
            let key = format!("{} {}", rule.method.to_ascii_uppercase(), rule.path_glob);
            if !api_declared.insert(key.clone()) {
                return Err(format!("api rule {key} is declared more than once"));
            }
        }

        let governed = self.tiers.get(&Tier::Governed).cloned().unwrap_or_default();
        for entry in &governed {
            let tuple = normalized_tuple(entry.tuple())?;
            let Some(canonical) = self.canonicalization.get(&tuple) else {
                return Err(format!("governed tuple {tuple} lacks canonicalization"));
            };
            if canonical.argv_forms.is_empty() || canonical.target_fields.is_empty() {
                return Err(format!(
                    "governed tuple {tuple} has incomplete canonicalization"
                ));
            }
        }
        for tuple in self.canonicalization.keys() {
            if declared.get(tuple) != Some(&Tier::Governed) {
                return Err(format!(
                    "canonicalization {tuple} does not name a governed tuple"
                ));
            }
        }

        for (repository, section) in &self.repository_sections {
            for removed in &section.removed_tuples {
                if !declared.contains_key(&normalized_tuple(removed)?) {
                    return Err(format!(
                        "repository section {repository} removes undeclared tuple {removed}"
                    ));
                }
            }
            for (tier, entries) in &section.tiers {
                for entry in entries {
                    let tuple = normalized_tuple(entry.tuple())?;
                    let Some(base) = declared.get(&tuple) else {
                        return Err(format!(
                            "repository section {repository} adds tuple {tuple}"
                        ));
                    };
                    if tier.rank() < base.rank() {
                        return Err(format!(
                            "repository section {repository} lowers tuple {tuple}"
                        ));
                    }
                }
            }
        }
        Ok(())
    }

    fn tier_for_tuple(&self, tuple: &str, platform: &str) -> Option<Tier> {
        self.tiers.iter().find_map(|(tier, entries)| {
            entries
                .iter()
                .any(|entry| {
                    normalized_tuple(entry.tuple()).ok().as_deref() == Some(tuple)
                        && platform_matches(entry.platform(), platform)
                })
                .then_some(*tier)
        })
    }
}

fn normalized_tuple(value: &str) -> Result<String, String> {
    let words = value
        .split_whitespace()
        .map(|word| word.to_ascii_lowercase())
        .collect::<Vec<_>>();
    (!words.is_empty())
        .then(|| words.join(" "))
        .ok_or_else(|| "tuple cannot be empty".to_string())
}

fn platform_matches(platforms: &[String], current: &str) -> bool {
    platforms
        .iter()
        .any(|platform| platform.eq_ignore_ascii_case(current))
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct SignedManifest {
    artifact_id: String,
    key_id: String,
    fetched_at_unix_secs: u64,
    signature: String,
    manifest: Manifest,
}

#[derive(Clone, Debug)]
enum ManifestProblem {
    Missing,
    Invalid(String),
    BelowFloor { manifest_floor: u64 },
    Stale { manifest_version: u64 },
}

impl ManifestProblem {
    fn diagnostic(&self) -> SelfReportDiagnostic {
        match self {
            Self::Missing => SelfReportDiagnostic::ManifestUnavailable,
            Self::Invalid(_) => SelfReportDiagnostic::ManifestInvalid,
            Self::BelowFloor { .. } => SelfReportDiagnostic::ManifestBelowFloor,
            Self::Stale { .. } => SelfReportDiagnostic::ManifestStale,
        }
    }

    fn status_label(&self) -> String {
        match self {
            Self::Missing => "unavailable".to_string(),
            Self::Invalid(error) => format!("invalid ({error})"),
            Self::BelowFloor { manifest_floor } => format!(
                "{} (manifest floor {manifest_floor}, shim floor {SCHEMA_FLOOR})",
                RefusalCode::ManifestBelowFloor.as_str()
            ),
            Self::Stale { manifest_version } => format!(
                "{} (manifest version {manifest_version})",
                RefusalCode::ManifestStale.as_str()
            ),
        }
    }
}

fn load_manifest(paths: &StatePaths, now: u64) -> Result<Manifest, ManifestProblem> {
    let bytes = fs::read(&paths.manifest).map_err(|_| ManifestProblem::Missing)?;
    let envelope: SignedManifest = serde_json::from_slice(&bytes)
        .map_err(|error| ManifestProblem::Invalid(error.to_string()))?;
    if envelope.artifact_id != MANIFEST_ARTIFACT_ID
        || envelope.manifest.artifact_id != MANIFEST_ARTIFACT_ID
    {
        return Err(ManifestProblem::Invalid("artifact id mismatch".to_string()));
    }
    verify_manifest_signature(&envelope)?;
    envelope
        .manifest
        .validate()
        .map_err(ManifestProblem::Invalid)?;
    if envelope.manifest.schema_floor < SCHEMA_FLOOR {
        return Err(ManifestProblem::BelowFloor {
            manifest_floor: envelope.manifest.schema_floor,
        });
    }
    if now.saturating_sub(envelope.fetched_at_unix_secs)
        > MANIFEST_TTL.as_secs() + MANIFEST_STALE_GRACE.as_secs()
    {
        return Err(ManifestProblem::Stale {
            manifest_version: envelope.manifest.manifest_version,
        });
    }
    Ok(envelope.manifest)
}

fn verify_manifest_signature(envelope: &SignedManifest) -> Result<(), ManifestProblem> {
    let Some(key) = trusted_manifest_key(&envelope.key_id) else {
        return Err(ManifestProblem::Invalid(format!(
            "untrusted manifest key id {}",
            envelope.key_id
        )));
    };
    let signature = base64::engine::general_purpose::STANDARD
        .decode(&envelope.signature)
        .map_err(|_| ManifestProblem::Invalid("invalid detached signature encoding".to_string()))?;
    let bytes = serde_json::to_vec(&envelope.manifest)
        .map_err(|error| ManifestProblem::Invalid(error.to_string()))?;
    UnparsedPublicKey::new(&ED25519, key)
        .verify(&bytes, &signature)
        .map_err(|_| ManifestProblem::Invalid("detached signature verification failed".to_string()))
}

// A manifest signature is the barrier preventing an agent from editing its own
// cache to turn a governed verb into a mechanical one. The development key is
// deliberately compiled only in debug builds so fixtures can exercise R3. A
// release build has no trust root until the separately reviewed CKCRED custody
// release supplies one, which keeps release binaries at R2 rather than making a
// governance claim with a test key.
#[cfg(not(debug_assertions))]
const RELEASE_MANIFEST_KEYS: &[(&str, &[u8])] = &[];

#[cfg(debug_assertions)]
const DEV_MANIFEST_KEY_ID: &str = "gh-routing-dev-test-key-v1";
#[cfg(debug_assertions)]
const DEV_MANIFEST_PUBLIC_KEY: [u8; 32] = [
    0xd7, 0x5a, 0x98, 0x01, 0x82, 0xb1, 0x0a, 0xb7, 0xd5, 0x4b, 0xfe, 0xd3, 0xc9, 0x64, 0x07, 0x3a,
    0x0e, 0xe1, 0x72, 0xf3, 0xda, 0xa6, 0x23, 0x25, 0xaf, 0x02, 0x1a, 0x68, 0xf7, 0x07, 0x51, 0x1a,
];

fn trusted_manifest_key(key_id: &str) -> Option<&'static [u8]> {
    #[cfg(debug_assertions)]
    if key_id == DEV_MANIFEST_KEY_ID {
        return Some(&DEV_MANIFEST_PUBLIC_KEY);
    }
    #[cfg(not(debug_assertions))]
    if let Some((_, key)) = RELEASE_MANIFEST_KEYS.iter().find(|(id, _)| *id == key_id) {
        return Some(*key);
    }
    let _ = key_id;
    None
}

#[derive(Debug)]
enum Classification {
    Mechanical,
    Governed {
        tuple: String,
        canonical: Canonicalization,
    },
    Admin {
        tuple: String,
    },
    Unclassified,
}

fn classify(args: &[OsString], manifest: &Manifest, platform: &str) -> Classification {
    let Some((verb, subcommand, _)) = command_head(args) else {
        return Classification::Unclassified;
    };
    if verb == "api" {
        return classify_api(args, manifest, platform);
    }
    let tuple = match subcommand {
        Some(subcommand) => format!("{verb} {subcommand}"),
        None => verb,
    };
    match manifest.tier_for_tuple(&tuple, platform) {
        Some(Tier::Mechanical) => Classification::Mechanical,
        Some(Tier::Admin) => Classification::Admin { tuple },
        Some(Tier::Governed) => manifest
            .canonicalization
            .get(&tuple)
            .cloned()
            .map(|canonical| Classification::Governed { tuple, canonical })
            .unwrap_or(Classification::Unclassified),
        None => Classification::Unclassified,
    }
}

fn command_head(args: &[OsString]) -> Option<(String, Option<String>, usize)> {
    let mut positionals = Vec::new();
    let mut skip_next = false;
    for (index, raw) in args.iter().enumerate() {
        let value = raw.to_str()?;
        if skip_next {
            skip_next = false;
            continue;
        }
        if matches!(value, "--repo" | "-R" | "--hostname" | "--config-dir") {
            skip_next = true;
            continue;
        }
        if value.starts_with('-') {
            continue;
        }
        positionals.push((value.to_ascii_lowercase(), index));
        if positionals.len() == 2 || positionals[0].0 == "api" {
            break;
        }
    }
    let (verb, index) = positionals.first()?.clone();
    let subcommand = positionals.get(1).map(|(value, _)| value.clone());
    Some((verb, subcommand, index))
}

fn classify_api(args: &[OsString], manifest: &Manifest, platform: &str) -> Classification {
    let Some((method, path)) = api_method_and_path(args) else {
        return Classification::Unclassified;
    };
    let matches = manifest
        .api_rules
        .iter()
        .filter(|rule| {
            rule.method.eq_ignore_ascii_case(&method)
                && platform_matches(&rule.platform, platform)
                && glob::Pattern::new(&rule.path_glob).is_ok_and(|pattern| pattern.matches(&path))
        })
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Classification::Unclassified;
    }
    match matches[0].tier {
        Tier::Mechanical => Classification::Mechanical,
        Tier::Admin => Classification::Admin {
            tuple: format!("api {method} {path}"),
        },
        // API governed declarations deliberately use their own canonicalization
        // key. A rule without one is safely unclassified instead of guessed.
        Tier::Governed => manifest
            .canonicalization
            .get(&format!("api {method} {path}"))
            .cloned()
            .map(|canonical| Classification::Governed {
                tuple: format!("api {method} {path}"),
                canonical,
            })
            .unwrap_or(Classification::Unclassified),
    }
}

fn api_method_and_path(args: &[OsString]) -> Option<(String, String)> {
    let mut method = "GET".to_string();
    let mut path = None;
    let mut index = 1;
    while index < args.len() {
        let value = args[index].to_str()?;
        if matches!(value, "--method" | "-X") {
            method = args.get(index + 1)?.to_str()?.to_ascii_uppercase();
            index += 2;
            continue;
        }
        if let Some(method_value) = value.strip_prefix("--method=") {
            method = method_value.to_ascii_uppercase();
            index += 1;
            continue;
        }
        if matches!(value, "--input" | "-F" | "-f" | "--raw-field" | "--field") {
            // Stdin and request-field forms can encode an undeclared body action.
            // They are not guessed from a read-like URL.
            return None;
        }
        if value.starts_with('-') {
            index += 1;
            continue;
        }
        if path.is_none() {
            path = Some(value.to_string());
        }
        index += 1;
    }
    let path = path?;
    (path != "-").then_some((method, path))
}

#[derive(Clone, Debug)]
struct GovernedRequest {
    action: String,
    target: Map<String, Value>,
    body: Map<String, Value>,
    repository: Option<String>,
    manifest_version: u64,
}

fn canonicalize_governed(
    args: &[OsString],
    tuple: &str,
    canonical: &Canonicalization,
    manifest_version: u64,
) -> Result<GovernedRequest, String> {
    let (_, _, head_index) =
        command_head(args).ok_or_else(|| "missing command head".to_string())?;
    let subcommand_index = if tuple.starts_with("api ") {
        head_index
    } else {
        head_index + 1
    };
    let mut positional = Vec::new();
    let mut body = Map::new();
    let mut explicit_repository = None;
    let mut index = subcommand_index + 1;
    while index < args.len() {
        let value = args[index]
            .to_str()
            .ok_or_else(|| "non-UTF-8 governed arguments are undeclared".to_string())?;
        if value == "--repo" || value == "-R" {
            index += 1;
            let repository = args
                .get(index)
                .and_then(|arg| arg.to_str())
                .ok_or_else(|| "--repo requires a value".to_string())?;
            explicit_repository = Some(repository.to_string());
        } else if let Some(repository) = value.strip_prefix("--repo=") {
            explicit_repository = Some(repository.to_string());
        } else if let Some((field, supplied)) =
            declared_body_value(value, canonical, args.get(index + 1))?
        {
            body.insert(field, Value::String(supplied));
            if !value.contains('=') && !value.starts_with('-') {
                // Kept for completeness; declared_body_value only returns flags.
                positional.push(value.to_string());
            }
            if !value.contains('=') {
                index += 1;
            }
        } else if value.starts_with('-') {
            return Err(format!("undeclared flag {value}"));
        } else {
            positional.push(value.to_string());
        }
        index += 1;
    }

    if positional.len() != canonical.target_fields.len() {
        return Err("target positional form is undeclared".to_string());
    }
    if canonical
        .body_fields
        .iter()
        .any(|field| !body.contains_key(field))
    {
        return Err("required declared body field is absent".to_string());
    }
    let target = canonical
        .target_fields
        .iter()
        .cloned()
        .zip(positional)
        .map(|(field, value)| (field, Value::String(value)))
        .collect::<Map<_, _>>();
    // A global `--repo` may precede the command head, so inspect the original
    // argv before falling back to a command-local flag or remote inference.
    let repository = explicit_repo(args)
        .or(explicit_repository)
        .or_else(infer_repository_from_git);
    Ok(GovernedRequest {
        action: tuple.to_string(),
        target,
        body,
        repository,
        manifest_version,
    })
}

fn declared_body_value(
    value: &str,
    canonical: &Canonicalization,
    next: Option<&OsString>,
) -> Result<Option<(String, String)>, String> {
    for field in &canonical.body_fields {
        let long = format!("--{field}");
        let short = match field.as_str() {
            "body" => Some("-b"),
            "reaction" => Some("-r"),
            _ => None,
        };
        if value == long || short == Some(value) {
            let supplied = next
                .and_then(|arg| arg.to_str())
                .ok_or_else(|| format!("{value} requires a value"))?;
            return Ok(Some((field.clone(), supplied.to_string())));
        }
        if let Some(supplied) = value.strip_prefix(&(long + "=")) {
            return Ok(Some((field.clone(), supplied.to_string())));
        }
    }
    Ok(None)
}

fn explicit_repo(args: &[OsString]) -> Option<String> {
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let value = arg.to_str()?;
        if value == "--repo" || value == "-R" {
            return args.next()?.to_str().map(str::to_string);
        }
        if let Some(repository) = value.strip_prefix("--repo=") {
            return Some(repository.to_string());
        }
    }
    None
}

fn infer_repository_from_git() -> Option<String> {
    let output = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()
        .map(|remote| remote.trim().to_string())
        .filter(|remote| !remote.is_empty())
}

#[derive(Debug)]
enum RouteOutcome {
    Result(String),
    Refusal(String),
    UnboundIdentity,
    SchemaMismatch(String),
    Unavailable(String),
}

fn route_governed(
    paths: &StatePaths,
    determination: &RungRecord,
    request: GovernedRequest,
) -> RouteOutcome {
    let Some(connection_file) = configured_connection_file() else {
        return RouteOutcome::Unavailable(
            "the governance connection file is no longer available".to_string(),
        );
    };
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let project_root = project_root_for(&cwd);
    let record_paths = paths.clone();
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => return RouteOutcome::Unavailable(error.to_string()),
    };
    runtime
        .block_on(async move {
            let options = ConsumerOptions {
                call_timeout: Duration::from_secs(5),
                ..ConsumerOptions::default()
            };
            let consumer = SubcConsumer::connect(&connection_file, options)
                .await
                .map_err(|error| RouteOutcome::Unavailable(error.to_string()))?;
            let catalog = consumer
                .catalog_list()
                .await
                .map_err(|error| RouteOutcome::Unavailable(error.to_string()))?;
            let holder = route_holder(&catalog.modules);
            record_unexpected_gh_route_advertisers(&record_paths, &holder.unexpected_advertisers);
            let module_id = holder.module_id.ok_or_else(|| {
                RouteOutcome::Unavailable("no holder advertises gh.route".to_string())
            })?;
            let route = consumer
                .open_route(
                    RouteTarget::ManagementSurface { module_id },
                    BindIdentity {
                        project_root: project_root.to_string_lossy().into_owned().into(),
                        harness: "aft-gh-shim".to_string(),
                        session: format!("gh-shim-{}", std::process::id()),
                    },
                    CallOptions::default(),
                )
                .await
                .map_err(|_| RouteOutcome::UnboundIdentity)?;
            let wire_request = json!({
                "operation": ROUTING_OPERATION,
                "gh_route_schema": 1,
                "action": request.action,
                "target": request.target,
                "body": request.body,
                "repository": request.repository,
                "manifest_version": request.manifest_version,
                "rung_as_of_unix_secs": determination.as_of_unix_secs,
            });
            let body = serde_json::to_vec(&wire_request)
                .map_err(|error| RouteOutcome::SchemaMismatch(error.to_string()))?;
            let response = consumer
                .request(&route, body, CallOptions::default())
                .await
                .map_err(|error| RouteOutcome::Unavailable(error.to_string()));
            let _ = consumer
                .close_handle(&route, CloseRouteOptions::default())
                .await;
            let response = response?;
            parse_governed_response(&response)
        })
        .unwrap_or_else(|outcome| outcome)
}

fn parse_governed_response(bytes: &[u8]) -> Result<RouteOutcome, RouteOutcome> {
    let value: Value = serde_json::from_slice(bytes).map_err(|_| {
        RouteOutcome::SchemaMismatch(
            "governance seam returned malformed or non-UTF-8 JSON".to_string(),
        )
    })?;
    let object = value.as_object().ok_or_else(|| {
        RouteOutcome::SchemaMismatch("governance seam response must be an object".to_string())
    })?;
    match object.get("outcome").and_then(Value::as_str) {
        Some("result") => {
            let schema = object
                .get("gh_route_schema")
                .and_then(Value::as_u64)
                .ok_or_else(|| {
                    RouteOutcome::SchemaMismatch(
                        "governance seam omitted gh_route_schema".to_string(),
                    )
                })?;
            if schema > 1 {
                return Err(RouteOutcome::SchemaMismatch(format!(
                    "governance seam schema {schema} is newer than supported schema 1"
                )));
            }
            let result = object.get("result").ok_or_else(|| {
                RouteOutcome::SchemaMismatch("governance seam omitted result".to_string())
            })?;
            let field_order = object
                .get("field_order")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    RouteOutcome::SchemaMismatch("governance seam omitted field_order".to_string())
                })?;
            render_governed_response(result, field_order).map(RouteOutcome::Result)
        }
        Some("refusal") => Ok(RouteOutcome::Refusal(
            object
                .get("refusal_code")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    RouteOutcome::SchemaMismatch(
                        "governance refusal omitted refusal_code".to_string(),
                    )
                })?
                .to_string(),
        )),
        Some("unbound_identity") => Ok(RouteOutcome::UnboundIdentity),
        _ => Err(RouteOutcome::SchemaMismatch(
            "governance seam returned an unknown outcome".to_string(),
        )),
    }
}

fn render_governed_response(result: &Value, field_order: &[Value]) -> Result<String, RouteOutcome> {
    let object = result.as_object().ok_or_else(|| {
        RouteOutcome::SchemaMismatch("governance result must be an object".to_string())
    })?;
    let mut output = String::new();
    let mut rendered = BTreeSet::new();
    for field in field_order {
        let field = field.as_str().ok_or_else(|| {
            RouteOutcome::SchemaMismatch("field_order must contain string fields".to_string())
        })?;
        let value = object.get(field).ok_or_else(|| {
            RouteOutcome::SchemaMismatch(format!(
                "field_order references absent result field {field}"
            ))
        })?;
        if !rendered.insert(field) {
            return Err(RouteOutcome::SchemaMismatch(format!(
                "field_order repeats result field {field}"
            )));
        }
        render_field(&mut output, field, value)?;
    }
    if rendered.len() != object.len() {
        return Err(RouteOutcome::SchemaMismatch(
            "field_order does not cover every governed result field".to_string(),
        ));
    }
    Ok(output)
}

fn render_field(output: &mut String, field: &str, value: &Value) -> Result<(), RouteOutcome> {
    match value {
        Value::Array(values) => {
            output.push_str(field);
            output.push_str(":\n");
            for value in values {
                output.push_str("  ");
                output.push_str(&render_scalar(value)?);
                output.push('\n');
            }
        }
        _ => {
            output.push_str(field);
            output.push_str(": ");
            output.push_str(&render_scalar(value)?);
            output.push('\n');
        }
    }
    Ok(())
}

fn render_scalar(value: &Value) -> Result<String, RouteOutcome> {
    match value {
        Value::String(value) => serde_json::to_string(value)
            .map_err(|error| RouteOutcome::SchemaMismatch(error.to_string())),
        Value::Number(_) | Value::Bool(_) | Value::Null => Ok(value.to_string()),
        Value::Object(_) | Value::Array(_) => serde_json::to_string(value)
            .map_err(|error| RouteOutcome::SchemaMismatch(error.to_string())),
    }
}

fn append_bypass_audit(
    paths: &StatePaths,
    tuple: &str,
    repository: Option<&str>,
    now: u64,
) -> io::Result<()> {
    fs::create_dir_all(&paths.root)?;
    let mut record = serde_json::to_vec(&json!({
        "as_of_unix_secs": now,
        "tuple": tuple,
        "repository": repository,
    }))
    .map_err(io::Error::other)?;
    record.push(b'\n');
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&paths.bypass_audit)?;
    file.write_all(&record)?;
    // An operator bypass is allowed only after the audit record is durable enough
    // to survive a process replacement. If this returns an error we do not exec.
    file.sync_data()
}

#[derive(Serialize)]
struct SelfReport {
    shim_version: &'static str,
    gh_routing_schema_floor: u64,
    unexpected_gh_route_advertiser: Option<Vec<String>>,
    cached_manifest: CachedManifestReport,
    last_rung: LastRungReport,
    bypass_audit: Option<Vec<Value>>,
    bypass_audit_error: Option<String>,
    executing_image: Option<String>,
    executing_image_error: Option<String>,
    real_gh_resolution: Option<RealGhResolution>,
    real_gh_resolution_error: Option<String>,
}

#[derive(Serialize)]
struct CachedManifestReport {
    version: Option<u64>,
    version_error: Option<String>,
    state: Option<&'static str>,
    state_error: Option<String>,
    diagnostics: Vec<&'static str>,
}

#[derive(Serialize)]
struct LastRungReport {
    rung: Option<&'static str>,
    rung_error: Option<String>,
    as_of_unix_secs: Option<u64>,
    as_of_unix_secs_error: Option<String>,
    determination_inputs: Option<BTreeMap<String, String>>,
    determination_inputs_error: Option<String>,
}

#[derive(Serialize)]
struct RealGhResolution {
    path: String,
    shim_path_positions: Vec<usize>,
}

fn print_self_report(paths: &StatePaths) {
    // This is deliberately one JSON document, rather than status lines, so a
    // later forensic process can consume it with jq while every dependency is down.
    if let Ok(document) = render_self_report(paths) {
        let mut stdout = io::stdout().lock();
        let _ = stdout.write_all(document.as_bytes());
    }
}

fn render_self_report(paths: &StatePaths) -> Result<String, serde_json::Error> {
    let report = build_self_report(paths);
    let mut document = serde_json::to_string(&report)?;
    document.push('\n');
    Ok(document)
}

fn build_self_report(paths: &StatePaths) -> SelfReport {
    let image = self_report_executing_image();
    let (real_gh_resolution, real_gh_resolution_error) = match image.as_ref() {
        Ok(image) => match resolve_real_gh(image) {
            Some(path) => (
                Some(RealGhResolution {
                    path: path.to_string_lossy().into_owned(),
                    shim_path_positions: executing_image_path_positions(image),
                }),
                None,
            ),
            None => (
                None,
                Some(
                    "PATH contains no upstream gh after skipping the executing shim image"
                        .to_string(),
                ),
            ),
        },
        Err(error) => (None, Some(format!("executing image unavailable: {error}"))),
    };
    let (bypass_audit, bypass_audit_error) = read_bypass_audit(paths);
    SelfReport {
        shim_version: env!("CARGO_PKG_VERSION"),
        gh_routing_schema_floor: SCHEMA_FLOOR,
        unexpected_gh_route_advertiser: unexpected_gh_route_advertisers(paths),
        cached_manifest: cached_manifest_report(paths),
        last_rung: last_rung_report(paths),
        bypass_audit,
        bypass_audit_error,
        executing_image: image
            .as_ref()
            .ok()
            .map(|path| path.to_string_lossy().into_owned()),
        executing_image_error: image.err(),
        real_gh_resolution,
        real_gh_resolution_error,
    }
}

fn cached_manifest_report(paths: &StatePaths) -> CachedManifestReport {
    match load_manifest(paths, unix_seconds()) {
        Ok(manifest) => CachedManifestReport {
            version: Some(manifest.manifest_version),
            version_error: None,
            state: Some("valid"),
            state_error: None,
            diagnostics: Vec::new(),
        },
        Err(problem) => {
            let error = problem.status_label();
            CachedManifestReport {
                version: None,
                version_error: Some(error.clone()),
                state: None,
                state_error: Some(error),
                diagnostics: vec![problem.diagnostic().as_str()],
            }
        }
    }
}

fn last_rung_report(paths: &StatePaths) -> LastRungReport {
    match fs::read(&paths.rung) {
        Ok(bytes) => match serde_json::from_slice::<RungRecord>(&bytes) {
            Ok(record) => LastRungReport {
                rung: Some(record.rung.label()),
                rung_error: None,
                as_of_unix_secs: Some(record.as_of_unix_secs),
                as_of_unix_secs_error: None,
                determination_inputs: Some(record.inputs),
                determination_inputs_error: None,
            },
            Err(error) => unavailable_last_rung(format!("corrupt rung cache: {error}")),
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            unavailable_last_rung("rung cache is unavailable".to_string())
        }
        Err(error) => unavailable_last_rung(format!("rung cache is unavailable: {error}")),
    }
}

fn unavailable_last_rung(error: String) -> LastRungReport {
    LastRungReport {
        rung: None,
        rung_error: Some(error.clone()),
        as_of_unix_secs: None,
        as_of_unix_secs_error: Some(error.clone()),
        determination_inputs: None,
        determination_inputs_error: Some(error),
    }
}

fn read_bypass_audit(paths: &StatePaths) -> (Option<Vec<Value>>, Option<String>) {
    let contents = match fs::read_to_string(&paths.bypass_audit) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return (Some(Vec::new()), None),
        Err(error) => return (None, Some(format!("bypass audit is unavailable: {error}"))),
    };
    let mut records = Vec::new();
    for (line_number, line) in contents.lines().enumerate() {
        match serde_json::from_str(line) {
            Ok(record) => records.push(record),
            Err(error) => {
                return (
                    None,
                    Some(format!(
                        "bypass audit is corrupt at line {}: {error}",
                        line_number + 1
                    )),
                )
            }
        }
    }
    (Some(records), None)
}

fn unexpected_gh_route_advertisers(paths: &StatePaths) -> Option<Vec<String>> {
    serde_json::from_slice(&fs::read(&paths.unexpected_gh_route_advertisers).ok()?)
        .ok()
        .filter(|advertisers: &Vec<String>| !advertisers.is_empty())
}

fn record_unexpected_gh_route_advertisers(paths: &StatePaths, advertisers: &[String]) {
    if advertisers.is_empty() {
        return;
    }
    let mut recorded = unexpected_gh_route_advertisers(paths)
        .unwrap_or_default()
        .into_iter()
        .collect::<BTreeSet<_>>();
    recorded.extend(advertisers.iter().cloned());
    let Ok(bytes) = serde_json::to_vec(&recorded.into_iter().collect::<Vec<_>>()) else {
        return;
    };
    let _ = fs::create_dir_all(&paths.root);
    let temporary = paths.unexpected_gh_route_advertisers.with_extension("tmp");
    if fs::write(&temporary, bytes).is_ok() {
        let _ = fs::rename(temporary, &paths.unexpected_gh_route_advertisers);
    }
}

fn self_report_executing_image() -> Result<PathBuf, String> {
    let path = std::env::current_exe().map_err(|error| error.to_string())?;
    Ok(path.canonicalize().unwrap_or(path))
}

fn executing_image() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.canonicalize().ok().or(Some(path)))
        .unwrap_or_else(|| PathBuf::from("unavailable"))
}

fn executing_image_path_positions(image: &Path) -> Vec<usize> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    std::env::split_paths(&path)
        .enumerate()
        .filter_map(|(index, directory)| same_image(&directory.join("gh"), image).then_some(index))
        .collect()
}

fn delegate(args: &[OsString]) -> i32 {
    let image = executing_image();
    let Some(real_gh) = resolve_real_gh(&image) else {
        return refuse(
            RefusalCode::NoRealGh,
            "PATH contains no upstream gh after skipping the executing shim image",
        );
    };
    exec_real_gh(real_gh, args)
}

fn resolve_real_gh(executing_image: &Path) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).find_map(|directory| {
        let candidate = directory.join("gh");
        (is_executable_file(&candidate) && !same_image(&candidate, executing_image))
            .then_some(candidate)
    })
}

fn is_executable_file(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        return fs::metadata(path).is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0);
    }
    #[cfg(not(unix))]
    true
}

fn same_image(left: &Path, right: &Path) -> bool {
    let left_canonical = left.canonicalize().ok();
    let right_canonical = right.canonicalize().ok();
    if left_canonical.is_some() && left_canonical == right_canonical {
        return true;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if let (Ok(left), Ok(right)) = (fs::metadata(left), fs::metadata(right)) {
            return left.dev() == right.dev() && left.ino() == right.ino();
        }
    }
    false
}

#[cfg(unix)]
fn exec_real_gh(real_gh: PathBuf, args: &[OsString]) -> i32 {
    use std::os::unix::process::CommandExt;
    let error = Command::new(real_gh).args(args).exec();
    // `exec` returns only if a candidate disappeared after the PATH scan. This
    // remains a shim refusal, rather than silently treating a failed exec as a
    // successful no-op.
    refuse(
        RefusalCode::NoRealGh,
        &format!("unable to exec upstream gh: {error}"),
    )
}

#[cfg(not(unix))]
fn exec_real_gh(real_gh: PathBuf, args: &[OsString]) -> i32 {
    match Command::new(real_gh).args(args).status() {
        Ok(status) => status.code().unwrap_or(1),
        Err(error) => refuse(
            RefusalCode::NoRealGh,
            &format!("unable to exec upstream gh: {error}"),
        ),
    }
}

fn refuse(code: RefusalCode, text: &str) -> i32 {
    let text = text.replace(['\n', '\r'], " ");
    eprintln!("gh-shim: {}: {text}", code.as_str());
    REFUSAL_EXIT_STATUS
}

fn current_platform() -> &'static str {
    if cfg!(target_os = "macos") {
        "macos"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else {
        "unsupported"
    }
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::{Ed25519KeyPair, KeyPair};

    const TEST_SEED: [u8; 32] = [
        0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec, 0x2c,
        0xc4, 0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03, 0x1c, 0xae,
        0x7f, 0x60,
    ];

    fn fixture_manifest() -> Manifest {
        serde_json::from_str(include_str!(
            "../tests/fixtures/gh_shim/initial-manifest-v1.json"
        ))
        .expect("initial manifest fixture")
    }

    fn signed(manifest: Manifest, fetched_at_unix_secs: u64) -> SignedManifest {
        let key = Ed25519KeyPair::from_seed_unchecked(&TEST_SEED).expect("test key");
        assert_eq!(key.public_key().as_ref(), DEV_MANIFEST_PUBLIC_KEY);
        let bytes = serde_json::to_vec(&manifest).expect("manifest bytes");
        SignedManifest {
            artifact_id: MANIFEST_ARTIFACT_ID.to_string(),
            key_id: DEV_MANIFEST_KEY_ID.to_string(),
            fetched_at_unix_secs,
            signature: base64::engine::general_purpose::STANDARD.encode(key.sign(&bytes).as_ref()),
            manifest,
        }
    }

    fn write_signed_manifest(paths: &StatePaths, manifest: Manifest, now: u64) {
        fs::create_dir_all(&paths.root).expect("state root");
        fs::write(
            &paths.manifest,
            serde_json::to_vec(&signed(manifest, now)).expect("signed manifest"),
        )
        .expect("manifest cache");
    }

    #[test]
    fn shim_dispatch_precedes_global_argument_scans_for_both_forms() {
        assert!(is_shim_invocation(
            OsStr::new("gh"),
            &[OsString::from("--version")]
        ));
        assert!(is_shim_invocation(
            OsStr::new("aft"),
            &[OsString::from("gh-shim"), OsString::from("--version")]
        ));
        assert!(!is_shim_invocation(
            OsStr::new("aft"),
            &[OsString::from("--version")]
        ));
    }

    #[test]
    fn reserved_self_report_tokens_are_exactly_the_two_first_arguments() {
        assert_eq!(RESERVED_SELF_REPORT, ["--status", "--shim-version"]);
        assert!(is_reserved_self_report(&[OsString::from("--status")]));
        assert!(is_reserved_self_report(&[OsString::from("--shim-version")]));
        assert!(!is_reserved_self_report(&[OsString::from("status")]));
        assert!(!is_reserved_self_report(&[
            OsString::from("issue"),
            OsString::from("--status")
        ]));
    }

    #[test]
    fn status_serializes_one_json_document_with_the_exact_top_level_schema() {
        let directory = tempfile::tempdir().unwrap();
        let paths = StatePaths::from_root(directory.path().to_path_buf());
        let document = render_self_report(&paths).expect("self report serialization");
        assert!(document.ends_with('\n'));
        let value: Value = serde_json::from_str(&document).expect("self report JSON");
        let keys = value
            .as_object()
            .expect("self report object")
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(
            keys,
            vec![
                "shim_version",
                "gh_routing_schema_floor",
                "unexpected_gh_route_advertiser",
                "cached_manifest",
                "last_rung",
                "bypass_audit",
                "bypass_audit_error",
                "executing_image",
                "executing_image_error",
                "real_gh_resolution",
                "real_gh_resolution_error",
            ]
        );
    }

    #[test]
    fn route_holder_is_pinned_and_records_other_advertisers() {
        let holder = select_route_holder([
            "other-module".to_string(),
            ROUTING_HOLDER_MODULE_ID.to_string(),
            "another-module".to_string(),
        ]);
        assert_eq!(holder.module_id.as_deref(), Some(ROUTING_HOLDER_MODULE_ID));
        assert_eq!(
            holder.unexpected_advertisers,
            vec!["another-module", "other-module"]
        );

        let holder = select_route_holder(["other-module".to_string()]);
        assert_eq!(holder.module_id, None);
        assert_eq!(holder.unexpected_advertisers, vec!["other-module"]);
    }

    #[test]
    fn unexpected_route_advertisers_are_persisted_for_self_report() {
        let directory = tempfile::tempdir().unwrap();
        let paths = StatePaths::from_root(directory.path().to_path_buf());
        record_unexpected_gh_route_advertisers(&paths, &["other-module".to_string()]);
        record_unexpected_gh_route_advertisers(&paths, &["another-module".to_string()]);

        assert_eq!(
            unexpected_gh_route_advertisers(&paths),
            Some(vec![
                "another-module".to_string(),
                "other-module".to_string(),
            ])
        );
        assert_eq!(
            build_self_report(&paths).unexpected_gh_route_advertiser,
            Some(vec![
                "another-module".to_string(),
                "other-module".to_string(),
            ])
        );
    }

    #[test]
    fn xdg_connection_config_precedes_home_config() {
        let directory = tempfile::tempdir().unwrap();
        let xdg = directory.path().join("xdg");
        let home = directory.path().join("home");
        let xdg_connection = directory.path().join("xdg-connection.json");
        let home_connection = directory.path().join("home-connection.json");
        fs::write(&xdg_connection, "{}").unwrap();
        fs::write(&home_connection, "{}").unwrap();
        let xdg_config = xdg.join("cortexkit/aft.jsonc");
        let home_config = home.join(".config/cortexkit/aft.jsonc");
        fs::create_dir_all(xdg_config.parent().unwrap()).unwrap();
        fs::create_dir_all(home_config.parent().unwrap()).unwrap();
        fs::write(
            &xdg_config,
            format!(
                r#"{{"subc":{{"connection_file":"{}"}}}}"#,
                xdg_connection.display()
            ),
        )
        .unwrap();
        fs::write(
            &home_config,
            format!(
                r#"{{"subc":{{"connection_file":"{}"}}}}"#,
                home_connection.display()
            ),
        )
        .unwrap();

        assert_eq!(
            configured_connection_file_from(Some(xdg.as_os_str()), Some(home.as_os_str())),
            Some(xdg_connection)
        );
    }

    #[test]
    fn initial_manifest_is_complete_and_valid() {
        fixture_manifest()
            .validate()
            .expect("valid initial manifest");
    }

    #[test]
    fn manifest_rejects_duplicate_tiers_and_empty_api_rationales() {
        let mut duplicate = fixture_manifest();
        duplicate
            .tiers
            .get_mut(&Tier::Admin)
            .unwrap()
            .push(TupleDecl::Details {
                tuple: "issue comment".to_string(),
                platform: vec!["macos".to_string()],
                api_match: None,
                rationale: None,
            });
        assert!(duplicate.validate().unwrap_err().contains("both"));

        let mut empty_api = fixture_manifest();
        empty_api
            .tiers
            .get_mut(&Tier::Admin)
            .unwrap()
            .push(TupleDecl::Details {
                tuple: "api patch close".to_string(),
                platform: vec!["macos".to_string()],
                api_match: Some(String::new()),
                rationale: None,
            });
        assert!(empty_api.validate().unwrap_err().contains("rationale"));
    }

    #[test]
    fn manifest_rejects_repo_sections_that_add_or_lower_a_tuple() {
        let mut manifest = fixture_manifest();
        manifest.repository_sections.insert(
            "owner/repo".to_string(),
            RepositorySection {
                tiers: BTreeMap::from([(
                    Tier::Mechanical,
                    vec![TupleDecl::Details {
                        tuple: "issue comment".to_string(),
                        platform: vec!["macos".to_string()],
                        api_match: None,
                        rationale: None,
                    }],
                )]),
                removed_tuples: Vec::new(),
            },
        );
        assert!(manifest.validate().unwrap_err().contains("lowers"));

        manifest.repository_sections.insert(
            "owner/repo".to_string(),
            RepositorySection {
                tiers: BTreeMap::from([(
                    Tier::Admin,
                    vec![TupleDecl::Details {
                        tuple: "workflow dispatch".to_string(),
                        platform: vec!["macos".to_string()],
                        api_match: None,
                        rationale: None,
                    }],
                )]),
                removed_tuples: Vec::new(),
            },
        );
        assert!(manifest.validate().unwrap_err().contains("adds"));
    }

    #[test]
    fn signed_cache_rejects_tampering_staleness_and_old_schema_floor() {
        let directory = tempfile::tempdir().unwrap();
        let paths = StatePaths::from_root(directory.path().to_path_buf());
        let now = 1_000_000;
        write_signed_manifest(&paths, fixture_manifest(), now);
        assert_eq!(load_manifest(&paths, now).unwrap().manifest_version, 1);

        let mut value: Value = serde_json::from_slice(&fs::read(&paths.manifest).unwrap()).unwrap();
        value["manifest"]["tiers"]["mechanical"][0]["tuple"] =
            Value::String("issue comment".to_string());
        fs::write(&paths.manifest, serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(matches!(
            load_manifest(&paths, now),
            Err(ManifestProblem::Invalid(_))
        ));
        assert_eq!(
            cached_manifest_report(&paths).diagnostics,
            vec![SelfReportDiagnostic::ManifestInvalid.as_str()]
        );

        let mut below_floor = fixture_manifest();
        below_floor.schema_floor = 0;
        write_signed_manifest(&paths, below_floor, now);
        assert!(matches!(
            load_manifest(&paths, now),
            Err(ManifestProblem::BelowFloor { manifest_floor: 0 })
        ));

        write_signed_manifest(
            &paths,
            fixture_manifest(),
            now - MANIFEST_TTL.as_secs() - MANIFEST_STALE_GRACE.as_secs() - 1,
        );
        assert!(matches!(
            load_manifest(&paths, now),
            Err(ManifestProblem::Stale { .. })
        ));
    }

    #[test]
    fn classification_is_allowlist_driven_without_a_write_heuristic() {
        let manifest = fixture_manifest();
        assert!(matches!(
            classify(
                &[OsString::from("issue"), OsString::from("view")],
                &manifest,
                "macos"
            ),
            Classification::Mechanical
        ));
        assert!(matches!(
            classify(
                &[OsString::from("api"), OsString::from("/repos/a/b")],
                &manifest,
                "macos"
            ),
            Classification::Mechanical
        ));
        assert!(matches!(
            classify(
                &[
                    OsString::from("api"),
                    OsString::from("--method=POST"),
                    OsString::from("/repos/a/b")
                ],
                &manifest,
                "macos"
            ),
            Classification::Unclassified
        ));
        assert!(matches!(
            classify(
                &[
                    OsString::from("api"),
                    OsString::from("--method"),
                    OsString::from("POST"),
                    OsString::from("/repos/a/b")
                ],
                &manifest,
                "macos"
            ),
            Classification::Unclassified
        ));
        assert!(matches!(
            classify(
                &[OsString::from("alias"), OsString::from("set")],
                &manifest,
                "macos"
            ),
            Classification::Unclassified
        ));
        assert!(matches!(
            classify(
                &[
                    OsString::from("alias"),
                    OsString::from("set"),
                    OsString::from("--write")
                ],
                &manifest,
                "macos"
            ),
            Classification::Unclassified
        ));
    }

    #[test]
    fn governed_canonicalization_normalizes_flags_and_explicit_repo_wins() {
        let manifest = fixture_manifest();
        let canonical = manifest.canonicalization["issue comment"].clone();
        let request = canonicalize_governed(
            &[
                OsString::from("--repo=owner/explicit"),
                OsString::from("issue"),
                OsString::from("comment"),
                OsString::from("42"),
                OsString::from("--body"),
                OsString::from("hello"),
            ],
            "issue comment",
            &canonical,
            1,
        )
        .unwrap();
        assert_eq!(request.repository.as_deref(), Some("owner/explicit"));
        assert_eq!(request.target["number"], "42");
        assert_eq!(request.body["body"], "hello");
    }

    #[test]
    fn governed_renderer_is_deterministic_for_scalars_arrays_and_escapes() {
        let result = json!({"message":"snowman ☃\n", "items":["a", 2], "ok":true});
        let order = vec![json!("ok"), json!("message"), json!("items")];
        assert_eq!(
            render_governed_response(&result, &order).unwrap(),
            "ok: true\nmessage: \"snowman ☃\\n\"\nitems:\n  \"a\"\n  2\n"
        );
        assert!(matches!(
            render_governed_response(&json!("scalar"), &order),
            Err(RouteOutcome::SchemaMismatch(_))
        ));
    }

    #[test]
    fn lower_rungs_are_cached_durably_but_r1_is_not_written() {
        let directory = tempfile::tempdir().unwrap();
        let paths = StatePaths::from_root(directory.path().to_path_buf());
        let record = RungRecord::r2(123, "daemon_unreachable", None);
        write_rung_record_silently(&paths, &record);
        assert_eq!(load_rung_record(&paths).unwrap().rung, Rung::R2);
        assert!(!paths.root.join("r1-cache.json").exists());
    }

    #[cfg(unix)]
    #[test]
    fn resolved_image_identity_skips_a_shim_reached_through_a_symlinked_parent() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let image = directory.path().join("aft");
        fs::write(&image, b"shim image").unwrap();
        let bin = directory.path().join("bin");
        fs::create_dir(&bin).unwrap();
        symlink(&image, bin.join("gh")).unwrap();
        let linked_parent = directory.path().join("linked-bin");
        symlink(&bin, &linked_parent).unwrap();

        assert!(same_image(&linked_parent.join("gh"), &image));
    }

    #[test]
    fn bypass_audit_is_visible_to_a_later_self_report_reader() {
        let directory = tempfile::tempdir().unwrap();
        let paths = StatePaths::from_root(directory.path().to_path_buf());
        append_bypass_audit(&paths, "issue close", Some("owner/repo"), 99).unwrap();
        let (records, error) = read_bypass_audit(&paths);
        assert!(error.is_none());
        let records = records.unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0]["tuple"], "issue close");
    }

    #[test]
    fn refusal_and_self_report_codes_are_separate_closed_sets() {
        assert_eq!(RefusalCode::ALL.len(), 10);
        assert!(RefusalCode::ALL
            .iter()
            .all(|code| code.as_str().starts_with("gh_shim_")));
        assert_eq!(SelfReportDiagnostic::ALL.len(), 5);
        assert!(SelfReportDiagnostic::ALL
            .iter()
            .all(|code| code.as_str().starts_with("gh_shim_status_")));
        assert_eq!(REFUSAL_EXIT_STATUS, 86);
    }
}
