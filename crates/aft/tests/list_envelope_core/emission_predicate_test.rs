use aft::list_envelope::{render_trailer, ListEnvelope, Reason, Total, Unit};

#[test]
fn trailer_renders_iff_reason_is_some() {
    // When reason is Some, trailer renders
    let env_with_reason = ListEnvelope::new(
        15,
        Total::Exact(21),
        Unit::Sites,
        vec![Reason::Cap],
        &["depth", "includeTests"],
    );
    assert!(env_with_reason.reason.is_some());
    assert!(render_trailer(&env_with_reason).is_some());

    // When reason is None (e.g. enumeration exhausted exactly on cap), trailer renders nothing
    let env_exhausted = ListEnvelope::new(
        20,
        Total::Exact(20),
        Unit::Sites,
        vec![],
        &["depth", "includeTests"],
    );
    assert!(env_exhausted.reason.is_none());
    assert!(env_exhausted.causes.is_empty());
    assert!(render_trailer(&env_exhausted).is_none());
}

#[test]
fn enumeration_exhausted_on_cap_serializes_no_reason_key() {
    let env = ListEnvelope::new(
        20,
        Total::Exact(20),
        Unit::Items,
        vec![],
        &["depth", "includeTests"],
    );
    let serialized = serde_json::to_value(&env).expect("serialization");
    assert!(
        serialized.get("reason").is_none(),
        "reason key must not be serialized when None: {serialized}"
    );
    assert_eq!(serialized["shown"], 20);
    assert_eq!(serialized["total"]["value"], 20);
    assert_eq!(serialized["total"]["kind"], "exact");
    assert_eq!(serialized["causes"], serde_json::json!([]));
}
