use aft::list_envelope::{render_trailer, ListEnvelope, Reason, Total, Unit};
use aft::list_surfaces::{find_surface, LIST_SURFACES};

#[test]
fn trailer_grammar_exact_with_narrow() {
    let env = ListEnvelope::new(
        15,
        Total::Exact(412),
        Unit::Sites,
        vec![Reason::Cap],
        &["depth", "includeTests"],
    );
    let rendered = render_trailer(&env).expect("trailer should render");
    assert_eq!(
        rendered,
        "shown 15 of 412 sites (cap) · narrow: depth, includeTests"
    );
    assert!(!rendered.ends_with('.'));
    assert!(!rendered.ends_with(';'));
    let prefix = rendered.split(" · narrow:").next().unwrap();
    assert!(!prefix.contains(',')); // no thousands separator in counts
}

#[test]
fn trailer_grammar_at_least_prefixed_geq() {
    let env = ListEnvelope::new(
        15,
        Total::AtLeast(15),
        Unit::Items,
        vec![Reason::Depth],
        &["depth", "includeTests"],
    );
    let rendered = render_trailer(&env).expect("trailer should render");
    assert_eq!(
        rendered,
        "shown 15 of ≥15 items (depth) · narrow: depth, includeTests"
    );
}

#[test]
fn trailer_grammar_zero_counts_at_least() {
    let env = ListEnvelope::new(
        0,
        Total::AtLeast(0),
        Unit::Paths,
        vec![Reason::Depth],
        &["depth", "includeTests"],
    );
    let rendered = render_trailer(&env).expect("trailer should render");
    assert_eq!(
        rendered,
        "shown 0 of ≥0 paths (depth) · narrow: depth, includeTests"
    );
}

#[test]
fn trailer_grammar_omits_narrow_clause_only_for_empty_narrow() {
    // bash compressed output has narrow: []
    let bash_surface = find_surface("bash", "", "bash.output").expect("bash surface");
    assert!(bash_surface.narrow.is_empty());

    let env = ListEnvelope::new(
        61,
        Total::Exact(4000),
        bash_surface.unit,
        vec![Reason::Cap],
        bash_surface.narrow,
    );
    let rendered = render_trailer(&env).expect("trailer should render");
    assert_eq!(rendered, "shown 61 of 4000 lines (cap)");
    assert!(!rendered.contains("narrow"));
    assert!(!rendered.contains("·"));
}

#[test]
fn trailer_grammar_for_every_registered_surface_matches_table_unit() {
    for surface in LIST_SURFACES {
        let first_reason = surface.reasons.first().expect("at least one reason");
        let env = ListEnvelope::new(
            10,
            Total::Exact(25),
            surface.unit,
            vec![first_reason.reason],
            surface.narrow,
        );
        let rendered = render_trailer(&env).expect("rendered trailer");

        // Verify the unit word in text equals envelope.unit and surface.unit
        let expected_unit = surface.unit.as_str();
        assert_eq!(env.unit.as_str(), expected_unit);
        assert!(
            rendered.contains(&format!(" 25 {expected_unit} ")),
            "rendered trailer `{rendered}` must contain unit word `{expected_unit}`"
        );

        if surface.narrow.is_empty() {
            assert!(
                !rendered.contains("narrow:"),
                "narrow clause must be omitted when narrow is empty for {}",
                surface.list_id
            );
        } else {
            let expected_narrow = surface.narrow.join(", ");
            assert!(
                rendered.contains(&format!("· narrow: {expected_narrow}")),
                "rendered trailer `{rendered}` must match knob order `{expected_narrow}`"
            );
        }
    }
}
