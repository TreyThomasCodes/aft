use aft::context::StatusBarCountValues;

#[test]
fn truthful_count_shape_keeps_proven_values_and_omits_unproven_categories() {
    let values = StatusBarCountValues {
        errors: None,
        warnings: None,
        dead_code: Some(7),
        unused_exports: None,
        duplicates: None,
        todos: None,
        tier2_stale: true,
    };

    assert_eq!(values.dead_code, Some(7));
    assert_eq!(values.errors, None);
    assert_eq!(values.warnings, None);
    assert_eq!(values.unused_exports, None);
    assert_eq!(values.duplicates, None);
    assert_eq!(values.todos, None);
}

#[test]
fn truthful_counts_and_inspect_payload_stay_out_of_agent_response_transport_seams() {
    let context = include_str!("../src/context.rs");
    let inspect = include_str!("../src/commands/inspect.rs");
    let response_finalize = include_str!("../src/response_finalize.rs");
    let subc_format = include_str!("../src/subc_format.rs");
    let main = include_str!("../src/main.rs");

    assert!(context.contains("pub struct StatusBarCountValues"));
    assert!(context.contains("pub errors: Option<usize>"));
    assert!(context.contains("pub todos: Option<usize>"));
    assert!(!context.contains("todos: tier2.todos.unwrap_or(0)"));
    assert!(!context.contains("match (tier2.dead_code, tier2.unused_exports, tier2.duplicates)"));

    assert!(
        !response_finalize.contains("status_bar_count_values"),
        "response finalization may consume only the temporary legacy projection"
    );
    assert!(
        !subc_format.contains("status_bar_count_values"),
        "subc formatting may not bypass response finalization with truthful counts"
    );
    assert!(
        !main.contains("status_bar_count_values"),
        "the main transport dispatch may not inject truthful counts directly"
    );
    assert!(inspect.contains("let payload = build_inspect_payload("));

    let payload_builder = inspect
        .find("fn build_inspect_payload")
        .expect("inspect payload builder exists");
    let payload_end = inspect[payload_builder..]
        .find("fn render_inspect_text")
        .map(|offset| payload_builder + offset)
        .expect("inspect text renderer follows the payload builder");
    let inspect_payload = &inspect[payload_builder..payload_end];
    assert!(
        !inspect_payload.contains("status_bar_count_values"),
        "aft_inspect responses are produced by build_inspect_payload, not count values"
    );
}

#[test]
fn inspect_outcomes_continue_to_feed_the_fleet_segment_without_freezing_text() {
    let inspect = include_str!("../src/commands/inspect.rs");
    let response_finalize = include_str!("../src/response_finalize.rs");

    assert!(inspect.contains("refresh_status_bar_counts(ctx, &outcomes);"));
    assert!(inspect.contains("ctx.update_status_bar_tier2("));
    // The counts refresh must run on the OUTCOMES side of the freshness gate:
    // truthful fleet values update from whatever the collection proved even when
    // `fresh_payloads` refuses the payload. (During parallel campaign assembly
    // this test asserted the freshness machinery was absent entirely; the two
    // campaigns now share this module, so the seam contract is ordering, not
    // absence.)
    let refresh_at = inspect
        .find("refresh_status_bar_counts(ctx, &outcomes);")
        .expect("counts refresh call site");
    let gate_at = inspect
        .find("let payloads = match fresh_payloads(&outcomes)")
        .expect("freshness gate call site");
    assert!(
        refresh_at < gate_at,
        "counts must refresh before the freshness gate can refuse the payload"
    );
    assert!(
        response_finalize.contains("let local_counts = ctx.status_bar_counts();")
            && response_finalize.contains(".map(aft_status_segment)"),
        "the fleet segment must still receive the values projection"
    );
}

#[test]
fn blocking_inspect_is_the_only_alert_observation_source_in_its_command_module() {
    let inspect = include_str!("../src/commands/inspect.rs");

    assert!(inspect.contains("AcceptedObservationBatch::from_diagnostic_snapshots("));
    assert_eq!(
        inspect.matches("accept_alert_observation_batch(").count(),
        1,
        "only the accepted blocking-inspect bridge may mutate alert state"
    );
}
