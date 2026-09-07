use aft::list_envelope::{render_trailer, ListEnvelope, Reason, Total, Unit};

#[test]
fn precedence_descending_order_and_reason_equals_first_cause() {
    // Test all combinations of causes to ensure Walk > Depth > Budget > Cap
    let all_causes = [Reason::Cap, Reason::Budget, Reason::Depth, Reason::Walk];

    // Cap and Budget -> Budget > Cap
    let env = ListEnvelope::new(
        10,
        Total::AtLeast(11),
        Unit::Results,
        vec![Reason::Cap, Reason::Budget],
        &["topK"],
    );
    assert_eq!(env.causes, vec![Reason::Budget, Reason::Cap]);
    assert_eq!(env.reason, Some(Reason::Budget));
    assert_eq!(env.reason, env.causes.first().copied());
    let text = render_trailer(&env).unwrap();
    assert!(text.contains("(budget)"));

    // Cap and Depth -> Depth > Cap
    let env = ListEnvelope::new(
        15,
        Total::AtLeast(15),
        Unit::Items,
        vec![Reason::Cap, Reason::Depth],
        &["depth"],
    );
    assert_eq!(env.causes, vec![Reason::Depth, Reason::Cap]);
    assert_eq!(env.reason, Some(Reason::Depth));
    assert_eq!(env.reason, env.causes.first().copied());
    let text = render_trailer(&env).unwrap();
    assert!(text.contains("(depth)"));

    // Budget and Depth -> Depth > Budget
    let env = ListEnvelope::new(
        15,
        Total::AtLeast(15),
        Unit::Paths,
        vec![Reason::Budget, Reason::Depth],
        &["depth"],
    );
    assert_eq!(env.causes, vec![Reason::Depth, Reason::Budget]);
    assert_eq!(env.reason, Some(Reason::Depth));
    assert_eq!(env.reason, env.causes.first().copied());
    let text = render_trailer(&env).unwrap();
    assert!(text.contains("(depth)"));

    // Walk and Cap -> Walk > Cap
    let env = ListEnvelope::new(
        100,
        Total::AtLeast(100),
        Unit::Rows,
        vec![Reason::Cap, Reason::Walk],
        &["path"],
    );
    assert_eq!(env.causes, vec![Reason::Walk, Reason::Cap]);
    assert_eq!(env.reason, Some(Reason::Walk));
    assert_eq!(env.reason, env.causes.first().copied());
    let text = render_trailer(&env).unwrap();
    assert!(text.contains("(walk)"));

    // Walk and Budget -> Walk > Budget
    let env = ListEnvelope::new(
        412,
        Total::AtLeast(412),
        Unit::Files,
        vec![Reason::Budget, Reason::Walk],
        &["path"],
    );
    assert_eq!(env.causes, vec![Reason::Walk, Reason::Budget]);
    assert_eq!(env.reason, Some(Reason::Walk));
    assert_eq!(env.reason, env.causes.first().copied());
    let text = render_trailer(&env).unwrap();
    assert!(text.contains("(walk)"));

    // Walk and Depth -> Walk > Depth
    let env = ListEnvelope::new(
        10,
        Total::AtLeast(10),
        Unit::Files,
        vec![Reason::Depth, Reason::Walk],
        &["path"],
    );
    assert_eq!(env.causes, vec![Reason::Walk, Reason::Depth]);
    assert_eq!(env.reason, Some(Reason::Walk));
    assert_eq!(env.reason, env.causes.first().copied());
    let text = render_trailer(&env).unwrap();
    assert!(text.contains("(walk)"));

    // All four causes together -> Walk > Depth > Budget > Cap
    let env = ListEnvelope::new(
        10,
        Total::AtLeast(10),
        Unit::Results,
        all_causes.to_vec(),
        &["topK"],
    );
    assert_eq!(
        env.causes,
        vec![Reason::Walk, Reason::Depth, Reason::Budget, Reason::Cap]
    );
    assert_eq!(env.reason, Some(Reason::Walk));
    assert_eq!(env.reason, env.causes.first().copied());
    let text = render_trailer(&env).unwrap();
    assert!(text.contains("(walk)"));
}
