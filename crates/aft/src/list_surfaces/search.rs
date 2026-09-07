//! Search list surface adapter.
//!
//! Handles list truncation envelope construction for `aft_search` (`payload.results`).
//!
//! Truncation causes per R8/R16:
//! - `engine_capped`: Bounding cut, bound `AtLeast(shown)`, reason `Budget`.
//! - `more_available`: Selecting cut, bound `AtLeast(shown + 1)`, reason `Cap`.
//! - Both flags: reason `Budget` (precedence over `Cap`), bound `AtLeast(shown + 1)`,
//!   `causes: ["budget", "cap"]`.
//! - Neither flag: complete answer, returns `None` (no trailer rendered, no envelope serialized).

use crate::list_envelope::{ListEnvelope, Reason, Total, Unit};

/// Command name associated with search in the surface registry.
pub const SEARCH_COMMAND: &str = "search";

/// Registered list ID for search results.
pub const SEARCH_LIST_ID: &str = "payload.results";

/// Permitted unit word for search results.
pub const SEARCH_UNIT: Unit = Unit::Results;

/// Narrowing parameters accepted by search in fixed render order.
pub const SEARCH_NARROW: &[&str] = &["topK", "path", "includeTests"];

/// Wire key for the search results envelope beside `results` in the payload (R14/R21).
pub const SEARCH_WIRE_KEY: &str = "results_list_envelope";

/// Construct the truncation envelope for `aft_search` results if truncated.
///
/// Returns `None` if neither `more_available` nor `engine_capped` is set,
/// meaning the result enumeration was complete.
pub fn build_search_envelope(
    shown: usize,
    more_available: bool,
    engine_capped: bool,
) -> Option<ListEnvelope> {
    if !more_available && !engine_capped {
        return None;
    }

    let mut causes = Vec::with_capacity(2);
    // Reason precedence: Budget (rank 2) > Cap (rank 1)
    if engine_capped {
        causes.push(Reason::Budget);
    }
    if more_available {
        causes.push(Reason::Cap);
    }

    // Bound per R8/R16:
    // - Selecting only (`more_available`): floor `shown + 1`
    // - Bounding only (`engine_capped`): bound `shown`
    // - Both: weakest bound wins / largest proved floor -> `shown + 1`
    let total = if more_available {
        Total::AtLeast(shown + 1)
    } else {
        Total::AtLeast(shown)
    };

    Some(ListEnvelope::new(
        shown,
        total,
        SEARCH_UNIT,
        causes,
        SEARCH_NARROW,
    ))
}

/// Attach the search truncation envelope to a JSON response object map.
///
/// If truncation occurred, serializes the envelope under `"results_list_envelope"`.
/// If the reply is complete, no envelope field is added.
pub fn attach_search_envelope(
    map: &mut serde_json::Map<String, serde_json::Value>,
    shown: usize,
    more_available: bool,
    engine_capped: bool,
) {
    if let Some(envelope) = build_search_envelope(shown, more_available, engine_capped) {
        if let Ok(val) = serde_json::to_value(envelope) {
            map.insert(SEARCH_WIRE_KEY.to_string(), val);
        }
    }
}
