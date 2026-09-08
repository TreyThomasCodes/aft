//! Grep list surface truncation adapter.
//!
//! Produces the truncation envelope for grep match lists (`payload.matches`),
//! accounting for directory walk boundaries (such as search timeouts or skipped
//! filesystem mounts) and match caps (such as executor result limits or
//! per-file display thinning).

use crate::list_envelope::{ListEnvelope, Reason, Total, Unit};
use serde_json::Value;

pub const COMMAND: &str = "grep";
pub const LIST_ID: &str = "payload.matches";
pub const UNIT: Unit = Unit::Rows;
pub const NARROW: &[&str] = &["path", "include", "exclude"];

/// Build the list truncation envelope for a grep response from its component parts.
///
/// Returns `None` if no truncation cause fired (the response is complete).
pub fn build_grep_envelope_from_parts(
    shown: usize,
    total_matches: usize,
    matches_count: usize,
    truncated: bool,
    walk_truncated: bool,
    skipped_foreign_mounts: usize,
) -> Option<ListEnvelope> {
    let walk_cause = walk_truncated || skipped_foreign_mounts > 0;
    let cap_cause = truncated || shown < matches_count;

    if !walk_cause && !cap_cause {
        return None;
    }

    let mut causes = Vec::new();
    if walk_cause {
        causes.push(Reason::Walk);
    }
    if cap_cause {
        causes.push(Reason::Cap);
    }

    let total = if walk_cause {
        // Any traversal cut means the true total is unknown, making the count a lower bound.
        Total::AtLeast(total_matches.max(shown))
    } else if truncated {
        // When the executor hits its result cap, the match count is a lower bound.
        Total::AtLeast(total_matches)
    } else {
        // When enumeration completed and only display thinning was applied, the total count is exact.
        Total::Exact(total_matches)
    };

    Some(ListEnvelope::new(shown, total, UNIT, causes, NARROW))
}

/// Build the list truncation envelope for a grep response JSON payload.
///
/// Inspects the response payload for walk boundaries and match caps, deriving
/// the rendered match count from the formatter seam. Returns `None` if the
/// result is complete.
pub fn build_grep_envelope(data: &Value) -> Option<ListEnvelope> {
    let matches = data.get("matches").and_then(Value::as_array);
    let matches_count = matches.map(|m| m.len()).unwrap_or(0);
    let total_matches = data
        .get("total_matches")
        .and_then(Value::as_u64)
        .map(|v| v as usize)
        .unwrap_or(matches_count)
        .max(matches_count);
    let truncated = data
        .get("truncated")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let walk_truncated = data
        .get("walk_truncated")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let skipped_foreign_mounts = data
        .get("skipped_foreign_mounts")
        .and_then(Value::as_u64)
        .map(|v| v as usize)
        .unwrap_or(0);

    let shown = crate::subc_format::rendered_grep_match_count(data);

    build_grep_envelope_from_parts(
        shown,
        total_matches,
        matches_count,
        truncated,
        walk_truncated,
        skipped_foreign_mounts,
    )
}
