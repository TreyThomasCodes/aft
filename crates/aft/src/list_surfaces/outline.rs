//! Outline list surface adapter.

use crate::list_envelope::{ListEnvelope, Reason, Total, Unit};

/// Measure trailer byte length for outline envelopes by wrapping `crate::list_envelope::measure_trailer_len`.
#[inline]
pub fn outline_trailer_byte_len(envelope: &crate::list_envelope::ListEnvelope) -> usize {
    crate::list_envelope::measure_trailer_len(envelope)
}

/// Authorized call-site for `measure_trailer_len` (R13).
#[inline]
pub fn outline_measure_trailer_len(envelope: &crate::list_envelope::ListEnvelope) -> usize {
    outline_trailer_byte_len(envelope)
}

/// Build list envelope for `aft_outline` files/directory mode per the truncation contract.
///
/// Causes:
/// - `Budget` (Selecting): fires iff budget rollups are present at the frontier.
/// - `Walk` (Bounding): fires iff directory traversal was incomplete (`collection_truncated`,
///   `walk_truncated`, or `skipped_foreign_mounts > 0`).
///
/// Precedence: Walk > Depth > Budget > Cap.
/// Total variant:
/// - `Exact(shown + budget_rollup_files)` if only Selecting causes fired.
/// - `AtLeast(shown + budget_rollup_files)` if any Bounding cause fired.
#[inline]
pub fn build_outline_files_envelope(
    shown: usize,
    budget_rollup_files: usize,
    budget_rollups_present: bool,
    collection_truncated: bool,
    walk_truncated: bool,
    skipped_foreign_mounts: usize,
) -> Option<ListEnvelope> {
    let mut causes = Vec::new();
    let walk_cause = collection_truncated || walk_truncated || skipped_foreign_mounts > 0;
    if walk_cause {
        causes.push(Reason::Walk);
    }
    if budget_rollups_present {
        causes.push(Reason::Budget);
    }

    if causes.is_empty() {
        return None;
    }

    let total = if walk_cause {
        Total::AtLeast(shown + budget_rollup_files)
    } else {
        Total::Exact(shown + budget_rollup_files)
    };

    Some(ListEnvelope::new(
        shown,
        total,
        Unit::Files,
        causes,
        &["path"],
    ))
}
