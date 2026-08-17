use std::path::Path;

use crate::context::AppContext;
use crate::protocol::{RawRequest, Response};

/// Handle the `edit_history` command: return the backup stack for a file.
///
/// Params: `file` (string, required) — path to query history for.
/// Returns: `{ file, entries: [{ backup_id, timestamp, description }, ...] }` (most recent last in stack order).
pub fn handle_edit_history(req: &RawRequest, ctx: &AppContext) -> Response {
    let file = match req.params.get("file").and_then(|v| v.as_str()) {
        Some(f) => f,
        None => {
            return Response::error(
                &req.id,
                "invalid_request",
                "edit_history: missing required param 'file'",
            );
        }
    };

    // Resolve relative paths against the bound project root so the backup key
    // matches the path the mutating tool recorded. A relative path passed
    // straight to `canonicalize_key` would be joined against the daemon's cwd
    // and miss the stack.
    let resolved = ctx.resolve_relative_path(Path::new(file));

    let backup = ctx.backup().lock();
    let history = backup.history(req.session(), &resolved);

    let entries: Vec<serde_json::Value> = history
        .iter()
        .rev() // Most recent first for the response
        .map(|entry| {
            serde_json::json!({
                "backup_id": entry.backup_id,
                "timestamp": entry.timestamp,
                "description": entry.description,
            })
        })
        .collect();

    Response::success(
        &req.id,
        serde_json::json!({
            "file": file,
            "entries": entries,
        }),
    )
}
