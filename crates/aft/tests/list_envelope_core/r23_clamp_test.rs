use aft::list_envelope::{render_trailer, ListEnvelope, Reason, Total, Unit};
use aft::list_surfaces::LIST_SURFACES;

#[test]
fn r23_clamp_at_999999999_exact_bare() {
    let env = ListEnvelope::new(
        500,
        Total::Exact(999_999_999),
        Unit::Results,
        vec![Reason::Cap],
        &["topK"],
    );
    let rendered = render_trailer(&env).expect("trailer");
    assert_eq!(
        rendered,
        "shown 500 of 999999999 results (cap) · narrow: topK"
    );

    // JSON preserves true value and true kind
    let json = serde_json::to_value(&env).expect("json");
    assert_eq!(json["total"]["value"], 999_999_999);
    assert_eq!(json["total"]["kind"], "exact");
}

#[test]
fn r23_clamp_at_999999999_at_least_prefixed_geq() {
    let env = ListEnvelope::new(
        500,
        Total::AtLeast(999_999_999),
        Unit::Results,
        vec![Reason::Budget],
        &["topK"],
    );
    let rendered = render_trailer(&env).expect("trailer");
    assert_eq!(
        rendered,
        "shown 500 of ≥999999999 results (budget) · narrow: topK"
    );

    let json = serde_json::to_value(&env).expect("json");
    assert_eq!(json["total"]["value"], 999_999_999);
    assert_eq!(json["total"]["kind"], "at_least");
}

#[test]
fn r23_clamp_at_1000000000_renders_geq_999999999_for_both_kinds() {
    // Exact(1_000_000_000) renders with ≥999999999
    let env_exact = ListEnvelope::new(
        100,
        Total::Exact(1_000_000_000),
        Unit::Rows,
        vec![Reason::Cap],
        &["path"],
    );
    let rendered_exact = render_trailer(&env_exact).expect("trailer");
    assert_eq!(
        rendered_exact,
        "shown 100 of ≥999999999 rows (cap) · narrow: path"
    );

    // JSON keeps true count and exact kind
    let json_exact = serde_json::to_value(&env_exact).expect("json");
    assert_eq!(json_exact["total"]["value"], 1_000_000_000);
    assert_eq!(json_exact["total"]["kind"], "exact");

    // AtLeast(1_000_000_000) renders with ≥999999999
    let env_at_least = ListEnvelope::new(
        100,
        Total::AtLeast(1_000_000_000),
        Unit::Rows,
        vec![Reason::Walk],
        &["path"],
    );
    let rendered_at_least = render_trailer(&env_at_least).expect("trailer");
    assert_eq!(
        rendered_at_least,
        "shown 100 of ≥999999999 rows (walk) · narrow: path"
    );

    // JSON keeps true count and at_least kind
    let json_at_least = serde_json::to_value(&env_at_least).expect("json");
    assert_eq!(json_at_least["total"]["value"], 1_000_000_000);
    assert_eq!(json_at_least["total"]["kind"], "at_least");
}

#[test]
fn shown_is_never_clamped() {
    // Even if shown is large, shown is never clamped
    let env = ListEnvelope::new(
        999_999_999,
        Total::Exact(999_999_999),
        Unit::Lines,
        vec![Reason::Cap],
        &[],
    );
    let rendered = render_trailer(&env).expect("trailer");
    assert_eq!(rendered, "shown 999999999 of 999999999 lines (cap)");
}

#[test]
fn worst_case_trailer_is_at_most_128_bytes() {
    // Worst case across all registered surfaces:
    // shown 999999999, total ≥999999999, longest reason (budget), longest knob list
    for surface in LIST_SURFACES {
        let env = ListEnvelope::new(
            999_999_999,
            Total::AtLeast(999_999_999),
            surface.unit,
            vec![Reason::Budget],
            surface.narrow,
        );
        let rendered = render_trailer(&env).expect("trailer");
        assert!(
            rendered.len() <= 128,
            "worst-case trailer for {} exceeded 128 bytes ({} bytes): `{}`",
            surface.list_id,
            rendered.len(),
            rendered
        );
    }
}
