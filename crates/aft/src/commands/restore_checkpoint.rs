use crate::checkpoint::{CHECKPOINT_DURABILITY, CHECKPOINT_RESTART_NOTICE};
use crate::context::AppContext;
use crate::error::AftError;
use crate::protocol::{RawRequest, Response};

/// Handle the `restore_checkpoint` command: restore files from a named checkpoint.
///
/// Params: `name` (string, required) — checkpoint name to restore.
/// Returns: `{ name, file_count, created_at, durability }` on success. A missing
/// checkpoint explains that named checkpoints do not survive bridge or daemon restarts.
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

    let checkpoint_store = ctx.checkpoint().lock();
    let file_paths = checkpoint_store
        .file_paths(req.session(), name)
        .map_err(|error| checkpoint_not_found_response(&req.id, error))?;
    let validated_paths = validate_restore_paths(&req.id, ctx, &file_paths)?;

    match checkpoint_store.restore_validated(req.session(), name, &validated_paths) {
        Ok(info) => Ok(Response::success(
            &req.id,
            serde_json::json!({
                "name": info.name,
                "file_count": info.file_count,
                "created_at": info.created_at,
                "durability": CHECKPOINT_DURABILITY,
            }),
        )),
        Err(e) => Ok(Response::error(&req.id, e.code(), e.to_string())),
    }
}

fn checkpoint_not_found_response(request_id: &str, error: AftError) -> Response {
    match error {
        AftError::CheckpointNotFound { name } => Response::error(
            request_id,
            "checkpoint_not_found",
            format!("checkpoint not found: {name}; {CHECKPOINT_RESTART_NOTICE}"),
        ),
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
