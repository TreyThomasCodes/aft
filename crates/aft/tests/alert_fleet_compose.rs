use std::collections::BTreeSet;

use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct CompositionFixture {
    assertions: Vec<GoldenAssertion>,
    fleet_line: String,
    without_fleet: ResponseGolden,
    with_fleet: ResponseGolden,
    shown_alert_identities: Vec<String>,
    counted_only_alert_identities: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ResponseGolden {
    response_text: String,
}

#[derive(Debug, Deserialize)]
struct GoldenAssertion {
    id: String,
    target: String,
}

fn fixture() -> CompositionFixture {
    serde_json::from_str(include_str!(
        "fixtures/alert_fleet_compose/composition.json"
    ))
    .expect("fleet-composition fixture is valid JSON")
}

fn retired_health_shape_in(line: &str) -> bool {
    let tokens = line.split_whitespace().collect::<Vec<_>>();
    tokens.windows(8).any(|window| {
        count_token(window[0], "E")
            && count_token(window[1], "W")
            && window[2] == "|"
            && count_token(window[3].strip_prefix('~').unwrap_or(window[3]), "D")
            && count_token(window[4], "U")
            && count_token(window[5], "C")
            && window[6] == "|"
            && count_token(window[7], "T")
    })
}

fn count_token(token: &str, prefix: &str) -> bool {
    token
        .strip_prefix(prefix)
        .map(|suffix| {
            let suffix = suffix.trim_end_matches(']');
            !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
        })
        .unwrap_or(false)
}

fn more_count(response_text: &str) -> Option<usize> {
    let suffixes = response_text
        .lines()
        .filter_map(|line| {
            line.strip_prefix("(+")
                .and_then(|line| line.strip_suffix(" more)"))
                .and_then(|count| count.parse::<usize>().ok())
        })
        .collect::<Vec<_>>();
    match suffixes.as_slice() {
        [count] => Some(*count),
        _ => None,
    }
}

fn validate_gate_three_no_freeze(assertions: &[GoldenAssertion]) -> Result<(), String> {
    for assertion in assertions {
        match assertion.target.as_str() {
            "alert-composition" | "fleet-values-computation" => {}
            "fleet-published-text" | "fleet-publish-condition" => {
                return Err(format!(
                    "Gate 3 forbids `{}` assertion `{}`",
                    assertion.target, assertion.id
                ));
            }
            target => return Err(format!("unknown golden target `{target}`")),
        }
    }
    Ok(())
}

#[test]
fn alert_block_composes_byte_identically_with_or_without_the_fleet_line() {
    let fixture = fixture();
    let without_fleet = &fixture.without_fleet.response_text;
    let with_fleet = &fixture.with_fleet.response_text;

    assert_eq!(
        without_fleet, with_fleet,
        "fleet publication must not alter the server-rendered alert bytes"
    );
    assert!(
        retired_health_shape_in(&fixture.fleet_line),
        "the exemption is meaningful only when the fleet line has the retained count shape"
    );
    for response in [without_fleet, with_fleet] {
        assert!(
            !response.lines().any(retired_health_shape_in),
            "retired health-count shapes are legal only in the fleet line"
        );
        assert!(
            response.contains("E0308"),
            "a diagnostic code is alert content, not a retired health-count token"
        );
    }
}

#[test]
fn more_suffix_counts_unshown_alert_identities() {
    let fixture = fixture();
    let all_identities = fixture
        .shown_alert_identities
        .iter()
        .chain(&fixture.counted_only_alert_identities)
        .collect::<BTreeSet<_>>();

    assert_eq!(
        all_identities.len(),
        fixture.shown_alert_identities.len() + fixture.counted_only_alert_identities.len(),
        "shown and counted-only identities must not overlap"
    );
    assert_eq!(
        more_count(&fixture.without_fleet.response_text),
        Some(fixture.counted_only_alert_identities.len()),
        "the suffix is a count of identities omitted from the rendered lines"
    );
}

#[test]
fn gate_three_no_freeze_uses_assertion_targets_not_descriptive_words() {
    let fixture = fixture();
    validate_gate_three_no_freeze(&fixture.assertions).expect("fixture has only legal goldens");

    let text_named_value_assertion = GoldenAssertion {
        id: "text mentions fleet values without freezing fleet output".to_string(),
        target: "fleet-values-computation".to_string(),
    };
    validate_gate_three_no_freeze(&[text_named_value_assertion])
        .expect("a descriptive word must not determine the policy");

    for target in ["fleet-published-text", "fleet-publish-condition"] {
        let error = validate_gate_three_no_freeze(&[GoldenAssertion {
            id: "value-chain continuity".to_string(),
            target: target.to_string(),
        }])
        .expect_err("Gate 3 must reject frozen fleet output assertions");
        assert!(error.contains(target));
    }
}
