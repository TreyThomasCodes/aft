use crate::checkpoint::CHECKPOINT_RESTART_NOTICE;
use crate::context::AppContext;
use crate::protocol::{RawRequest, Response};

/// Handle the `list_checkpoints` command: return metadata for all checkpoints.
///
/// No params required.
/// Returns: `{ checkpoints: [{ name, file_count, created_at }, ...], durability }`.
/// Checkpoints are memory-only, so the durability notice explains an empty list
/// after the bridge or daemon has restarted.
pub fn handle_list_checkpoints(req: &RawRequest, ctx: &AppContext) -> Response {
    let checkpoint_store = ctx.checkpoint().lock();
    let list = checkpoint_store.list(req.session());

    let checkpoints: Vec<serde_json::Value> = list
        .iter()
        .map(|info| {
            serde_json::json!({
                "name": info.name,
                "file_count": info.file_count,
                "created_at": info.created_at,
            })
        })
        .collect();

    Response::success(
        &req.id,
        serde_json::json!({
            "checkpoints": checkpoints,
            "durability": CHECKPOINT_RESTART_NOTICE,
        }),
    )
}
