//! NDJSON text builder for standalone command responses.
//!
//! Provides the authorized NDJSON call-site for `render_trailer` (R13).
//! If an envelope is present in the response, renders the trailer and suppresses
//! legacy clauses; if absent, preserves the pre-spec text byte-unchanged.

use crate::list_envelope::{derive_wire_key, render_trailer, ListEnvelope};
use serde_json::Value;

/// Render agent-facing text for an NDJSON response.
///
/// If an envelope is present for the given list surface, renders the trailer.
/// If absent, returns `base_text` unchanged.
pub fn build_ndjson_text(
    base_text: &str,
    data: &Value,
    list_id: Option<&str>,
    is_text_surface: bool,
) -> String {
    let Some(id) = list_id else {
        return base_text.to_string();
    };

    let wire_key = derive_wire_key(id, is_text_surface);
    let envelope_val = data
        .get(&wire_key)
        .or_else(|| data.get("payload").and_then(|p| p.get(&wire_key)))
        .or_else(|| {
            data.get("details")
                .or_else(|| data.get("payload").and_then(|p| p.get("details")))
                .and_then(|d| d.get(&wire_key))
        });

    let Some(val) = envelope_val else {
        return base_text.to_string();
    };

    let Ok(envelope) = serde_json::from_value::<ListEnvelope>(val.clone()) else {
        return base_text.to_string();
    };

    if is_text_surface {
        // For text surfaces such as bash, the trailer is already integrated
        // into the text payload; pass through unchanged.
        return base_text.to_string();
    }

    if let Some(trailer) = render_trailer(&envelope) {
        if base_text.is_empty() {
            trailer
        } else {
            format!("{base_text}\n\n{trailer}")
        }
    } else {
        base_text.to_string()
    }
}
