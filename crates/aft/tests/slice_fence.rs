use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;

const RESPONSE_FINALIZE: &str = "crates/aft/src/response_finalize.rs";
const MAIN_RS: &str = "crates/aft/src/main.rs";
const ALERT_FINALIZATION_SLICE: &str = "alert-finalization";
const HEALTH_DIGEST_SLICE: &str = "health-digest";
const HEALTH_DIGEST_REGISTRATION: &str = "health.digest registration arm";

#[derive(Clone, Debug, Deserialize)]
struct SliceMap {
    campaign_edits: Vec<String>,
    slices: Vec<Slice>,
    main_changes: Vec<MainChange>,
}

#[derive(Clone, Debug, Deserialize)]
struct Slice {
    id: String,
    files: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
struct MainChange {
    file: String,
    target: String,
}

fn fixture() -> SliceMap {
    serde_json::from_str(include_str!("fixtures/alert_fleet_compose/slice_map.json"))
        .expect("slice-map fixture is valid JSON")
}

fn validate_slice_map(map: &SliceMap) -> Result<(), String> {
    let campaign_edits = map.campaign_edits.iter().collect::<BTreeSet<_>>();
    if campaign_edits.len() != map.campaign_edits.len() {
        return Err("campaign edit list names a file more than once".to_string());
    }

    let mut owners = BTreeMap::new();
    for slice in &map.slices {
        if slice.id.trim().is_empty() {
            return Err("slice has an empty id".to_string());
        }
        for file in &slice.files {
            if !campaign_edits.contains(file) {
                return Err(format!("slice `{}` owns unedited file `{file}`", slice.id));
            }
            if let Some(previous) = owners.insert(file.as_str(), slice.id.as_str()) {
                return Err(format!(
                    "file `{file}` is owned by both `{previous}` and `{}`",
                    slice.id
                ));
            }
        }
    }

    for file in &map.campaign_edits {
        if !owners.contains_key(file.as_str()) {
            return Err(format!("campaign edit `{file}` is unowned"));
        }
    }

    if owners.get(RESPONSE_FINALIZE) != Some(&ALERT_FINALIZATION_SLICE) {
        return Err(format!(
            "`{RESPONSE_FINALIZE}` must be owned only by `{ALERT_FINALIZATION_SLICE}`"
        ));
    }
    if owners.get(MAIN_RS) != Some(&HEALTH_DIGEST_SLICE) {
        return Err(format!(
            "`{MAIN_RS}` must be owned only by `{HEALTH_DIGEST_SLICE}`"
        ));
    }

    match map.main_changes.as_slice() {
        [MainChange { file, target }]
            if file == MAIN_RS && target == HEALTH_DIGEST_REGISTRATION => {}
        _ => {
            return Err(format!(
                "`{MAIN_RS}` may change only through the single `{HEALTH_DIGEST_REGISTRATION}`"
            ));
        }
    }

    Ok(())
}

#[test]
fn campaign_slice_map_is_complete_and_non_overlapping() {
    validate_slice_map(&fixture()).expect("every campaign edit has exactly one owning slice");
}

#[test]
fn response_finalization_cannot_be_shared_by_another_slice() {
    let mut map = fixture();
    map.slices.push(Slice {
        id: "duplicate-finalization-owner".to_string(),
        files: vec![RESPONSE_FINALIZE.to_string()],
    });

    let error = validate_slice_map(&map).expect_err("finalization ownership must not overlap");
    assert!(error.contains(RESPONSE_FINALIZE));
    assert!(error.contains("duplicate-finalization-owner"));
}

#[test]
fn every_campaign_edit_requires_a_declared_owner() {
    let mut map = fixture();
    map.campaign_edits
        .push("crates/aft/src/unowned_campaign_edit.rs".to_string());

    let error = validate_slice_map(&map).expect_err("an unowned campaign edit must be rejected");
    assert!(error.contains("unowned_campaign_edit.rs"));
}

#[test]
fn main_allows_only_the_health_digest_registration_arm() {
    let mut map = fixture();
    map.main_changes.push(MainChange {
        file: MAIN_RS.to_string(),
        target: "unrelated dispatch change".to_string(),
    });

    let error = validate_slice_map(&map).expect_err("a second main.rs edit must be rejected");
    assert!(error.contains(HEALTH_DIGEST_REGISTRATION));

    let mut wrong_target = fixture();
    wrong_target.main_changes[0].target = "health digest help text".to_string();
    assert!(validate_slice_map(&wrong_target).is_err());
}
