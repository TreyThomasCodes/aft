use aft::list_envelope::derive_wire_key;
use aft::list_surfaces::LIST_SURFACES;
use serde_json::Value;

#[test]
fn wire_key_derivation_array_lists_vs_text_surfaces() {
    // Array lists get last-segment + _list_envelope
    assert_eq!(
        derive_wire_key("payload.sites", false),
        "sites_list_envelope"
    );
    assert_eq!(
        derive_wire_key("payload.callers", false),
        "callers_list_envelope"
    );
    assert_eq!(derive_wire_key("payload.tree", false), "tree_list_envelope");
    assert_eq!(
        derive_wire_key("payload.paths", false),
        "paths_list_envelope"
    );
    assert_eq!(derive_wire_key("payload.hops", false), "hops_list_envelope");
    assert_eq!(
        derive_wire_key("payload.results", false),
        "results_list_envelope"
    );
    assert_eq!(
        derive_wire_key("payload.matches", false),
        "matches_list_envelope"
    );
    assert_eq!(
        derive_wire_key("payload.files", false),
        "files_list_envelope"
    );
    assert_eq!(
        derive_wire_key("payload.details.dead_code", false),
        "dead_code_list_envelope"
    );

    // Text surfaces get full id with . -> _ plus _list_envelope
    assert_eq!(
        derive_wire_key("bash.output", true),
        "bash_output_list_envelope"
    );
}

fn assert_no_bare_list_envelope_key_recursive(val: &Value) {
    match val {
        Value::Object(map) => {
            assert!(
                !map.contains_key("list_envelope"),
                "found forbidden bare key 'list_envelope' in object: {val:#?}"
            );
            for value in map.values() {
                assert_no_bare_list_envelope_key_recursive(value);
            }
        }
        Value::Array(arr) => {
            for item in arr {
                assert_no_bare_list_envelope_key_recursive(item);
            }
        }
        _ => {}
    }
}

#[test]
fn recursive_assertion_no_bare_list_envelope_key() {
    for surface in LIST_SURFACES {
        let is_text = surface.command == "bash";
        let key = derive_wire_key(surface.list_id, is_text);
        assert_ne!(
            key, "list_envelope",
            "derived key must never be bare 'list_envelope'"
        );

        let simulated_reply = serde_json::json!({
            "id": "1",
            "success": true,
            "payload": {
                &key: {
                    "shown": 10,
                    "total": {"kind": "exact", "value": 20},
                    "unit": surface.unit.as_str(),
                    "reason": "cap",
                    "causes": ["cap"],
                    "narrow": surface.narrow,
                }
            }
        });

        assert_no_bare_list_envelope_key_recursive(&simulated_reply);
    }
}
