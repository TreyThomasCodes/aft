use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::context::AppContext;
use crate::harness::Harness;
use crate::hashline::integration::{
    display_files_from_envelope, effective_for_capture, hashline_preflight_from_args,
    render_mutation_response, render_rejection_response, MutationRenderInput, TransportKind,
};
use crate::hashline::syntax::{
    parse_hashline_patch, resolve_patch_sections, Baseline, HashlineRejection, Operation,
};
use crate::hashline::transaction::{
    run_transaction, ExecuteContext, MvDestinationInput, TransactionSectionInput,
};
use crate::protocol::{RawRequest, Response};

struct OwnedMvDestination {
    canonical_path: PathBuf,
    requested_path: String,
    baseline_bytes: Option<Vec<u8>>,
}

pub fn handle_preflight(req: &RawRequest, ctx: &AppContext) -> Response {
    let Some((_guard, root)) = effective_binding(req, ctx) else {
        return rejection_response(
            &req.id,
            &HashlineRejection::parse("hashline edit is not enabled for this session"),
            transport_kind(ctx),
        );
    };
    match hashline_preflight_from_args(&req.params, Some(&root)) {
        Ok(result) => Response::success(&req.id, result.to_json()),
        Err(rejection) => rejection_response(&req.id, &rejection, transport_kind(ctx)),
    }
}

pub fn handle_edit(req: &RawRequest, ctx: &AppContext) -> Response {
    let Some((guard, root)) = effective_binding(req, ctx) else {
        return rejection_response(
            &req.id,
            &HashlineRejection::parse("hashline edit is not enabled for this session"),
            transport_kind(ctx),
        );
    };
    let preview = req
        .params
        .get("preview")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mut agent_arguments = req.params.clone();
    if let Some(arguments) = agent_arguments.as_object_mut() {
        arguments.remove("preview");
    }
    let patch_text = match crate::hashline::syntax::validate_raw_arguments(&agent_arguments) {
        Ok(request) => request.patch,
        Err(rejection) => {
            return rejection_response(&req.id, &rejection, transport_kind(ctx));
        }
    };
    let patch = match parse_hashline_patch(&patch_text) {
        Ok(patch) => patch,
        Err(rejection) => {
            return rejection_response(&req.id, &rejection, transport_kind(ctx));
        }
    };

    let result = guard.with_binding_mut(|binding| {
        let resolved = resolve_patch_sections(binding.snapshots_mut(), &patch, |requested| {
            resolve_write_path(req, ctx, &root, requested)
        })?;

        let mut baselines = BTreeMap::<PathBuf, Baseline>::new();
        for section in &resolved {
            if !baselines.contains_key(&section.canonical_path) {
                let bytes = fs::read(&section.canonical_path).map_err(|error| {
                    HashlineRejection::untaggable_path(format!(
                        "failed to load {}: {error}",
                        section.canonical_path.display()
                    ))
                })?;
                baselines.insert(section.canonical_path.clone(), Baseline::from_bytes(bytes));
            }
        }

        let mut destinations = Vec::with_capacity(patch.sections.len());
        for section in &patch.sections {
            let destination = section
                .operations
                .iter()
                .find_map(|operation| match operation {
                    Operation::Mv(mv) => Some(mv.destination.as_str()),
                    _ => None,
                });
            let owned = if let Some(requested_path) = destination {
                let canonical_path = resolve_write_path(req, ctx, &root, requested_path)?;
                let baseline_bytes = if canonical_path.exists() {
                    Some(fs::read(&canonical_path).map_err(|error| {
                        HashlineRejection::untaggable_path(format!(
                            "failed to load MV destination {}: {error}",
                            canonical_path.display()
                        ))
                    })?)
                } else {
                    None
                };
                Some(OwnedMvDestination {
                    canonical_path,
                    requested_path: requested_path.to_string(),
                    baseline_bytes,
                })
            } else {
                None
            };
            destinations.push(owned);
        }

        let mut display_baselines = Vec::<(String, Vec<u8>)>::new();
        let inputs = resolved
            .iter()
            .zip(patch.sections.iter())
            .zip(destinations.iter())
            .map(|((resolved, section), destination)| {
                let baseline = baselines
                    .get(&resolved.canonical_path)
                    .expect("every resolved source has one baseline");
                display_baselines.push((
                    section.header.requested_path.clone(),
                    baseline.bytes.clone(),
                ));
                let mv_destination = destination.as_ref().map(|destination| {
                    if let Some(bytes) = destination.baseline_bytes.as_ref() {
                        display_baselines.push((destination.requested_path.clone(), bytes.clone()));
                    }
                    MvDestinationInput {
                        canonical_path: destination.canonical_path.as_path(),
                        requested_path: destination.requested_path.as_str(),
                        baseline_bytes: destination.baseline_bytes.as_deref(),
                    }
                });
                TransactionSectionInput {
                    canonical_path: resolved.canonical_path.as_path(),
                    requested_path: section.header.requested_path.as_str(),
                    baseline,
                    snapshot: &resolved.snapshot,
                    operations: section.operations.as_slice(),
                    resolved: resolved.operations.as_slice(),
                    mv_destination,
                }
            })
            .collect::<Vec<_>>();

        let register_snapshot = binding.registers().clone();
        let (snapshots, registers) = binding.stores_mut();
        let mut backups = ctx.backup().lock();
        let backups_enabled = backups.policy().enabled;
        let mut execute = ExecuteContext {
            session: req.session(),
            backups: &mut backups,
            snapshots,
            registers,
            backups_enabled,
            fault: None,
        };
        let envelope = run_transaction(&inputs, &register_snapshot, &mut execute, preview)?;
        let display_files = display_files_from_envelope(&envelope, &display_baselines);
        Ok((envelope, display_files))
    });

    match result {
        Ok((envelope, display_files)) => {
            let payload = render_mutation_response(MutationRenderInput {
                envelope: &envelope,
                display_files: &display_files,
                project_root: Some(&root),
                transport: transport_kind(ctx),
            });
            response_from_payload(&req.id, payload)
        }
        Err(rejection) => rejection_response(&req.id, &rejection, transport_kind(ctx)),
    }
}

fn effective_binding(
    req: &RawRequest,
    ctx: &AppContext,
) -> Option<(crate::hashline::integration::BindingGuard, PathBuf)> {
    let root = ctx
        .canonical_cache_root_opt()
        .or_else(|| ctx.config().project_root.clone())?;
    let guard = ctx
        .hashline_bindings()
        .capture(&root, req.session().to_string())?;
    effective_for_capture(Some(&guard)).then_some((guard, root))
}

fn resolve_write_path(
    req: &RawRequest,
    ctx: &AppContext,
    project_root: &Path,
    requested: &str,
) -> Result<PathBuf, HashlineRejection> {
    let path = crate::subc_translate::resolve_path_from_project_root(project_root, requested);
    ctx.validate_write_location(&req.id, &path)
        .map_err(|response| {
            let message = response
                .data
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("path is not write-eligible");
            HashlineRejection::untaggable_path(message)
        })
}

fn transport_kind(ctx: &AppContext) -> TransportKind {
    match ctx.harness_opt() {
        Some(Harness::Opencode) => TransportKind::OpenCode,
        Some(Harness::Pi) => TransportKind::Pi,
        Some(Harness::Mcp { .. }) => TransportKind::Mcp,
        Some(Harness::Runner | Harness::Fed { .. }) | None => TransportKind::Ndjson,
    }
}

fn rejection_response(
    id: &str,
    rejection: &HashlineRejection,
    transport: TransportKind,
) -> Response {
    response_from_payload(id, render_rejection_response(rejection, transport))
}

fn response_from_payload(id: &str, mut payload: Value) -> Response {
    let success = payload
        .get("success")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if let Some(object) = payload.as_object_mut() {
        object.remove("success");
    }
    Response {
        id: id.to_string(),
        success,
        data: payload,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    use crate::config::Config;
    use crate::context::default_language_provider_factory;
    use crate::hashline::integration::RegistrationRequest;
    use crate::protocol::{RawRequest, DEFAULT_SESSION_ID};

    fn request(command: &str, params: Value) -> RawRequest {
        RawRequest {
            id: format!("hashline-{command}-test"),
            command: command.to_string(),
            lsp_hints: None,
            session_id: None,
            params,
        }
    }

    #[test]
    fn preflight_then_apply_mutates_bytes_and_records_undo() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = std::fs::canonicalize(temp.path()).expect("canonical root");
        let path = root.join("sample.txt");
        std::fs::write(&path, "alpha\nbeta\n").expect("fixture write");
        let ctx = AppContext::new(
            default_language_provider_factory(),
            Config {
                project_root: Some(root.clone()),
                ..Default::default()
            },
        );
        let registration = ctx.hashline_bindings().register(
            &root,
            DEFAULT_SESSION_ID.to_string(),
            RegistrationRequest {
                configured_enabled: true,
                edit_slot_survives: true,
            },
        );
        assert!(registration.effective);

        let mut read = request("read", json!({ "file": path }));
        read.params["_hashline_requested_path"] = Value::String("sample.txt".to_string());
        let read_response = crate::commands::read::handle_read(&read, &ctx);
        let tag = read_response.data["hashline_tag"]
            .as_str()
            .expect("tagged read")
            .to_string();
        let patch = format!("*** Begin Patch\n[sample.txt#{tag}]\nPUT 1:\n+omega\n*** End Patch");

        let preflight = handle_preflight(
            &request("hashline_preflight", json!({ "patch": patch.clone() })),
            &ctx,
        );
        assert!(preflight.success, "{}", preflight.data);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "alpha\nbeta\n");

        let response = handle_edit(&request("hashline_edit", json!({ "patch": patch })), &ctx);
        assert!(response.success, "{}", response.data);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "omega\nbeta\n");
        assert!(response.data["op_id"].as_str().is_some());
        assert_eq!(
            ctx.backup().lock().history(DEFAULT_SESSION_ID, &path).len(),
            1
        );
    }

    #[test]
    fn server_preview_flag_is_not_validated_as_an_agent_argument() {
        let temp = tempfile::tempdir().expect("tempdir");
        let root = std::fs::canonicalize(temp.path()).expect("canonical root");
        let path = root.join("sample.txt");
        std::fs::write(&path, "alpha\nbeta\n").expect("fixture write");
        let ctx = AppContext::new(
            default_language_provider_factory(),
            Config {
                project_root: Some(root.clone()),
                ..Default::default()
            },
        );
        ctx.hashline_bindings().register(
            &root,
            DEFAULT_SESSION_ID.to_string(),
            RegistrationRequest {
                configured_enabled: true,
                edit_slot_survives: true,
            },
        );

        let mut read = request("read", json!({ "file": path }));
        read.params["_hashline_requested_path"] = Value::String("sample.txt".to_string());
        let read_response = crate::commands::read::handle_read(&read, &ctx);
        let tag = read_response.data["hashline_tag"]
            .as_str()
            .expect("tagged read");
        let patch = format!("[sample.txt#{tag}]\nPUT 1:\n+omega");

        let preview = handle_edit(
            &request("hashline_edit", json!({ "patch": patch, "preview": true })),
            &ctx,
        );
        assert!(preview.success, "{}", preview.data);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "alpha\nbeta\n");
        assert!(preview.data["preview"].as_bool().unwrap_or(false));
    }

    #[test]
    fn unregistered_hashline_handler_refuses_direct_invocation() {
        let temp = tempfile::tempdir().expect("tempdir");
        let ctx = AppContext::new(
            default_language_provider_factory(),
            Config {
                project_root: Some(temp.path().to_path_buf()),
                ..Default::default()
            },
        );
        let response = handle_edit(&request("hashline_edit", json!({ "patch": "x" })), &ctx);
        assert!(!response.success);
        assert_eq!(response.data["code"], "hashline_parse_error");
    }
}
