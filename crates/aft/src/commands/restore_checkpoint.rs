use crate::checkpoint::{
    checkpoint_durability, CHECKPOINT_HYDRATED_NOTICE, CHECKPOINT_RESTART_NOTICE,
};
use crate::context::AppContext;
use crate::error::AftError;
use crate::protocol::{RawRequest, Response};

/// Handle the `restore_checkpoint` command: restore files from a named checkpoint.
///
/// Params: `name` (string, required) — checkpoint name to restore.
/// Returns: `{ name, file_count, created_at, storage_path, durability }` on success.
/// A missing checkpoint only cites restart loss when durable hydration found no
/// checkpoints in the caller's session.
pub fn handle_restore_checkpoint(req: &RawRequest, ctx: &AppContext) -> Response {
    match handle_restore_checkpoint_impl(req, ctx) {
        Ok(resp) | Err(resp) => resp,
    }
}

fn handle_restore_checkpoint_impl(
    req: &RawRequest,
    ctx: &AppContext,
) -> Result<Response, Response> {
    let name = match req.params.get("name").and_then(|v| v.as_str()) {
        Some(n) => n,
        None => {
            return Ok(Response::error(
                &req.id,
                "invalid_request",
                "restore_checkpoint: missing required param 'name'",
            ));
        }
    };

    let mut checkpoint_store = ctx.checkpoint().lock();
    let file_paths = checkpoint_store
        .file_paths(req.session(), name)
        .map_err(|error| {
            checkpoint_not_found_response(
                &req.id,
                error,
                checkpoint_store.session_is_empty(req.session()),
            )
        })?;
    let validated_paths = validate_restore_paths(&req.id, ctx, &file_paths)?;

    match checkpoint_store.restore_validated(req.session(), name, &validated_paths) {
        Ok(info) => {
            let storage_path = info
                .storage_path
                .as_deref()
                .map(|path| path.display().to_string())
                .unwrap_or_default();
            Ok(Response::success(
                &req.id,
                serde_json::json!({
                    "name": info.name,
                    "file_count": info.file_count,
                    "created_at": info.created_at,
                    "storage_path": storage_path,
                    "durability": checkpoint_durability(std::path::Path::new(&storage_path)),
                }),
            ))
        }
        Err(e) => Ok(Response::error(&req.id, e.code(), e.to_string())),
    }
}

fn checkpoint_not_found_response(
    request_id: &str,
    error: AftError,
    session_is_empty: bool,
) -> Response {
    match error {
        AftError::CheckpointNotFound { name } => {
            let notice = if session_is_empty {
                CHECKPOINT_RESTART_NOTICE
            } else {
                CHECKPOINT_HYDRATED_NOTICE
            };
            Response::error(
                request_id,
                "checkpoint_not_found",
                format!("checkpoint not found: {name}; {notice}"),
            )
        }
        other => Response::error(request_id, other.code(), other.to_string()),
    }
}

fn validate_restore_paths(
    req_id: &str,
    ctx: &AppContext,
    file_paths: &[std::path::PathBuf],
) -> Result<Vec<std::path::PathBuf>, Response> {
    for path in file_paths {
        ctx.validate_write_location(req_id, path)?;
    }

    // Authorization must not replace the checkpoint key with a symlink target.
    // The restore writer removes a final-component symlink before materializing
    // the snapshot, so the stored location is both the lookup and write target.
    Ok(file_paths.to_vec())
}
