use super::protocol::*;
use serde_json::{Value, json};

fn schema(name: &str) -> jsonschema::Validator {
    let value: Value = serde_json::from_str(match name {
        "request" => include_str!("../../schemas/reswho-extension-request-1.schema.json"),
        "manifest" => include_str!("../../schemas/reswho-extension-manifest-1.schema.json"),
        _ => include_str!("../../schemas/reswho-extension-response-1.schema.json"),
    })
    .unwrap();
    jsonschema::validator_for(&value).unwrap()
}

#[test]
fn protocol_fixtures_parse_and_validate_without_snapshot_dtos() {
    let response = schema("response");
    for bytes in [
        include_str!("../../tests/fixtures/extensions/resources.json"),
        include_str!("../../tests/fixtures/extensions/resources-additive.json"),
        include_str!("../../tests/fixtures/extensions/role.json"),
        include_str!("../../tests/fixtures/extensions/containers.json"),
    ] {
        let value: Value = serde_json::from_str(bytes).unwrap();
        let errors: Vec<_> = response
            .iter_errors(&value)
            .map(|error| error.to_string())
            .collect();
        assert!(errors.is_empty(), "{errors:?}");
        let _: Response = serde_json::from_str(bytes).unwrap();
    }
    let base: Response = serde_json::from_str(include_str!(
        "../../tests/fixtures/extensions/resources.json"
    ))
    .unwrap();
    let additive: Response = serde_json::from_str(include_str!(
        "../../tests/fixtures/extensions/resources-additive.json"
    ))
    .unwrap();
    assert_eq!(
        serde_json::to_value(base).unwrap(),
        serde_json::to_value(additive).unwrap()
    );
    let request = include_str!("../../tests/fixtures/extensions/request.json");
    let _: Request = serde_json::from_str(request).unwrap();
    assert!(schema("request").is_valid(&serde_json::from_str::<Value>(request).unwrap()));
    let manifest = include_str!("../../tests/fixtures/extensions/manifest.json");
    let mut parsed: Manifest = serde_json::from_str(manifest).unwrap();
    parsed.validate().unwrap();
    assert!(schema("manifest").is_valid(&serde_json::from_str::<Value>(manifest).unwrap()));
    let mut constrained: Value = serde_json::from_str(manifest).unwrap();
    constrained["definitions"][0]["future_constraint"] = json!(true);
    assert!(serde_json::from_value::<Manifest>(constrained.clone()).is_err());
    assert!(!schema("manifest").is_valid(&constrained));
}

#[test]
fn resource_values_preserve_precision_and_reject_identity_metric_and_value_bounds() {
    let value: Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/extensions/resources.json"
    ))
    .unwrap();
    let Contribution::Collect { resources, .. } = serde_json::from_value::<Response>(value.clone())
        .unwrap()
        .contribution
    else {
        panic!("collect")
    };
    let accepted = super::resources::convert("fixture", "1.0.0", resources.unwrap()).unwrap();
    assert_eq!(
        accepted[0].metrics[0].value.as_ref().unwrap().to_string(),
        "9007199254740993"
    );
    assert_eq!(
        accepted[0].metrics[1].value.as_ref().unwrap().to_string(),
        "-2.5"
    );
    assert!(accepted[0].metrics[2].value.is_none());
    for case in [
        "duplicate_id",
        "duplicate_metric",
        "relative_path",
        "bad_unit",
        "bad_kind",
        "too_many_metrics",
        "infinite",
    ] {
        let mut bad = value.clone();
        let resources = bad["contribution"]["resources"].as_array_mut().unwrap();
        match case {
            "duplicate_id" => resources.push(resources[0].clone()),
            "duplicate_metric" => {
                let metric = resources[0]["metrics"][0].clone();
                resources[0]["metrics"].as_array_mut().unwrap().push(metric);
            }
            "relative_path" => resources[0]["path"] = json!("relative"),
            "bad_unit" => resources[0]["metrics"][0]["unit"] = json!("bad unit"),
            "bad_kind" => resources[0]["kind"] = json!(""),
            "too_many_metrics" => {
                resources[0]["metrics"] = json!(vec![resources[0]["metrics"][0].clone(); 65])
            }
            _ => resources[0]["metrics"][0]["value"] = serde_json::from_str("1e999").unwrap(),
        }
        let Contribution::Collect { resources, .. } = serde_json::from_value::<Response>(bad)
            .unwrap()
            .contribution
        else {
            panic!("collect")
        };
        assert!(
            super::resources::convert("fixture", "1.0.0", resources.unwrap()).is_err(),
            "{case}"
        );
    }
    let _: Response = serde_json::from_value(value.clone()).unwrap();
    let mut wrong = value;
    wrong["contribution"]["resources"][0]["metrics"][0]["value"] = json!("123");
    assert!(serde_json::from_value::<Response>(wrong.clone()).is_err());
    assert!(!schema("response").is_valid(&wrong));
}

#[test]
fn resource_scopes_render_project_and_sort_without_metric_aggregation() {
    let response: Response = serde_json::from_str(include_str!(
        "../../tests/fixtures/extensions/resources.json"
    ))
    .unwrap();
    let Contribution::Collect { resources, .. } = response.contribution else {
        panic!("collect")
    };
    let mut accepted =
        super::resources::convert("first", "1.0.0", resources.clone().unwrap()).unwrap();
    accepted.extend(super::resources::convert("second", "2.0.0", resources.unwrap()).unwrap());
    let root = super::resources::build(accepted);
    assert_ne!(
        root.children[0].children[0].reference,
        root.children[1].children[0].reference
    );
    assert_eq!(root.totals.processes, 0);
    assert_eq!(root.totals.resident_bytes, 0);
    assert!(root.container_totals.is_none());
    crate::tree::validate_references(&root).unwrap();
    let snapshot = crate::model::Snapshot {
        schema: crate::model::JSON_SCHEMA_VERSION,
        extensions: vec![],
        captured_unix_ms: 0,
        sample_ms: 1,
        collection_ms: 1,
        system: Default::default(),
        scope: root.reference.clone(),
        warnings: vec![],
        root: root.clone(),
    };
    let json = serde_json::to_value(&snapshot).unwrap();
    let schema: Value = serde_json::from_str(crate::output::JSON_SCHEMA).unwrap();
    let errors: Vec<_> = jsonschema::validator_for(&schema)
        .unwrap()
        .iter_errors(&json)
        .map(|error| error.to_string())
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
    let md = crate::output::markdown(&snapshot);
    assert!(md.contains("9007199254740993 bytes") && md.contains("-2.5 vendor.celsius"));
    let selected = crate::tree::select(&root, &root.children[0].children[0].reference).unwrap();
    let detailed = crate::model::Snapshot {
        root: selected,
        ..snapshot
    };
    assert!(crate::output::markdown(&detailed).contains("Used: ? bytes"));
    for sort in [
        crate::sort::Sort::Name,
        crate::sort::Sort::Cpu,
        crate::sort::Sort::Memory,
        crate::sort::Sort::Uptime,
    ] {
        let mut sorted = root.clone();
        sort.apply(&mut sorted);
        assert_eq!(sorted.totals.resident_bytes, 0);
        assert_eq!(sorted.children.len(), 2);
    }
    let projected = crate::output::markdown_with_depth(&detailed, 0);
    assert!(projected.contains("Data volume"));
}

#[test]
fn repair_typed_protocol_roundtrips_remain_schema_valid_for_all_scopes_and_variants() {
    let resources: Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/extensions/resources.json"
    ))
    .unwrap();
    let containers: Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/extensions/containers.json"
    ))
    .unwrap();
    let role: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/extensions/role.json")).unwrap();
    let mut dual = containers.clone();
    dual["contribution"]["resources"] = resources["contribution"]["resources"].clone();
    let details = json!({"protocol_version":1,"provider":"fixture","version":"1.0.0","contribution":{"capability":"parsed_details","identity":{"pid":42,"started_unix_us":123},"fields":[{"key":"extensions.fixture.value","label":"Value","value":null,"sensitive":false}]}});
    let response_schema = schema("response");
    for value in [resources, containers, dual, role, details] {
        let typed: Response = serde_json::from_value(value).unwrap();
        let encoded = serde_json::to_value(typed).unwrap();
        let errors: Vec<_> = response_schema
            .iter_errors(&encoded)
            .map(|error| error.to_string())
            .collect();
        assert!(errors.is_empty(), "{errors:?}");
        let _: Response = serde_json::from_value(encoded).unwrap();
    }
    let manifest: Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/extensions/manifest.json"
    ))
    .unwrap();
    for collect in [true, false] {
        let mut value = manifest.clone();
        if !collect {
            value["capabilities"] = json!(["classify"]);
            value.as_object_mut().unwrap().remove("collect_scopes");
        }
        let typed: Manifest = serde_json::from_value(value).unwrap();
        let encoded = serde_json::to_value(typed).unwrap();
        assert!(schema("manifest").is_valid(&encoded), "{encoded}");
    }
    for capability in ["collect", "classify", "details"] {
        let mut value: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/extensions/request.json"))
                .unwrap();
        value["capability"] = json!(capability);
        let process = json!({"pid":42,"started_unix_us":123,"ppid":null,"uid":null,"name":"fixture","executable":null,"cwd":null});
        if capability != "collect" {
            value.as_object_mut().unwrap().remove("collect_scopes");
        }
        if capability == "classify" {
            value["processes"] = json!([process]);
        } else if capability == "details" {
            value["process"] = process;
        }
        let typed: Request = serde_json::from_value(value).unwrap();
        let encoded = serde_json::to_value(typed).unwrap();
        assert!(schema("request").is_valid(&encoded), "{encoded}");
    }
}

#[test]
fn repair_protocol_integer_fields_accept_exact_decimal_and_exponent_values() {
    let baseline: Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/extensions/containers.json"
    ))
    .unwrap();
    let validator = schema("response");
    for text in [
        "1000.0",
        "1e3",
        "9007199254740993.0",
        "9007199254740993e0",
        "18446744073709551615.0",
        "1.8446744073709551615e19",
        "0e999",
        "-0.0",
    ] {
        let mut value = baseline.clone();
        value["contribution"]["containers"][0]["memory_bytes"] =
            serde_json::from_str(text).unwrap();
        assert!(
            validator.is_valid(&value),
            "schema: {text}: {:?}",
            validator
                .iter_errors(&value)
                .map(|error| error.to_string())
                .collect::<Vec<_>>()
        );
        let parsed: Response =
            serde_json::from_slice(&serde_json::to_vec(&value).unwrap()).unwrap();
        let Contribution::Collect { containers, .. } = parsed.contribution else {
            panic!("collect")
        };
        let expected = match text {
            "1000.0" | "1e3" => 1000,
            "9007199254740993.0" | "9007199254740993e0" => 9007199254740993,
            "0e999" | "-0.0" => 0,
            _ => u64::MAX,
        };
        assert_eq!(
            containers.unwrap()[0].memory_bytes,
            Some(expected),
            "{text}"
        );
    }
    for text in [
        "0.1",
        "1e-3",
        "-1.0",
        "18446744073709551616.0",
        "1.8446744073709551616e19",
        "1e999",
        "1.0000000000000000000001",
    ] {
        let mut value = baseline.clone();
        value["contribution"]["containers"][0]["memory_bytes"] =
            serde_json::from_str(text).unwrap();
        assert!(!validator.is_valid(&value), "schema accepted {text}");
        assert!(serde_json::from_value::<Response>(value).is_err(), "{text}");
    }
    let mut response = baseline;
    response["protocol_version"] = serde_json::from_str("1e0").unwrap();
    response["contribution"]["runtime"]["collection_ms"] = serde_json::from_str("1e3").unwrap();
    let container = &mut response["contribution"]["containers"][0];
    container["uptime_seconds"] = serde_json::from_str("1000.0").unwrap();
    container["ports"] =
        json!([{"protocol":"tcp","container_port":65535.0,"host_ip":null,"host_port":1e3}]);
    container["unavailable"] = json!([{"code":"unavailable","operation":"fixture","message":"safe","os_error":-2147483648.0}]);
    assert!(validator.is_valid(&response));
    let parsed: Response = serde_json::from_value(response).unwrap();
    let Contribution::Collect {
        runtime,
        containers,
        ..
    } = parsed.contribution
    else {
        panic!("collect")
    };
    assert_eq!(runtime.unwrap().collection_ms, 1000);
    let container = containers.unwrap().remove(0);
    assert_eq!(container.uptime_seconds, Some(1000));
    assert_eq!(container.ports.as_ref().unwrap()[0].host_port, Some(1000));
    assert_eq!(container.unavailable[0].os_error, Some(i32::MIN));
    let identity: Identity =
        serde_json::from_str("{\"pid\":4294967295.0,\"started_unix_us\":18446744073709551615.0}")
            .unwrap();
    assert_eq!(identity.pid, u32::MAX);
    assert_eq!(identity.started_unix_us, u64::MAX);
    assert!(
        serde_json::from_str::<Identity>("{\"pid\":4294967296.0,\"started_unix_us\":0}").is_err()
    );
    let observed: ObservedProcess = serde_json::from_str(
        "{\"pid\":42.0,\"started_unix_us\":1e3,\"ppid\":1.0,\"uid\":501.0,\"name\":\"fixture\"}",
    )
    .unwrap();
    assert_eq!(observed.ppid, Some(1));
    assert_eq!(observed.uid, Some(501));
}

#[test]
fn repair_resource_group_scope_survives_selection_and_depth_zero() {
    let response: Response = serde_json::from_str(include_str!(
        "../../tests/fixtures/extensions/resources.json"
    ))
    .unwrap();
    let Contribution::Collect { resources, .. } = response.contribution else {
        panic!("collect")
    };
    let root = super::resources::build(
        super::resources::convert("fixture", "1.0.0", resources.unwrap()).unwrap(),
    );
    let provider = &root.children[0];
    let leaf = &provider.children[0];
    for reference in [&root.reference, &provider.reference, &leaf.reference] {
        let selected = crate::tree::select(&root, reference).unwrap();
        let snapshot = crate::model::Snapshot {
            schema: crate::model::JSON_SCHEMA_VERSION,
            extensions: vec![],
            captured_unix_ms: 0,
            sample_ms: 1,
            collection_ms: 1,
            system: Default::default(),
            scope: reference.clone(),
            warnings: vec![],
            root: selected,
        };
        let md = crate::output::markdown_with_depth(&snapshot, 0);
        assert!(
            !md.contains("Σ0") && !md.contains("0.0%") && !md.contains("0 B"),
            "{md}"
        );
        if reference != &leaf.reference {
            assert!(md.contains("independent metrics"), "{md}");
        }
    }
    let empty = super::resources::build(vec![]);
    assert!(matches!(
        empty.grouping,
        Some(crate::model::Grouping::Resource)
    ));
}

#[test]
fn tags_wire_shapes_are_strict_bounded_and_exact_for_all_operations() {
    let identity = json!({"pid":42,"started_unix_us":18446744073709551615u64});
    for change in [
        json!({"operation":"add","tag":{"dimension":"role","id":"role:test","label":"Test","path":null,"basis":"observed"}}),
        json!({"operation":"exclude","selector":{"dimension":"role","tag_id":"role:test","source":"other"}}),
        json!({"operation":"withdraw","proposal_id":"add-test"}),
    ] {
        let value = json!({"protocol_version":1,"provider":"fixture","version":"1.0.0","required_features":["tags"],"contribution":{"capability":"classify","proposals":[{"identity":identity,"evidence":{"kind":"tags","proposal":{"id":"one","identity":identity,"change":change}}}]}});
        assert!(schema("response").is_valid(&value), "{value}");
        let parsed: Response = serde_json::from_value(value.clone()).unwrap();
        assert!(schema("response").is_valid(&serde_json::to_value(parsed).unwrap()));
        let mut invalid = value;
        invalid["contribution"]["proposals"][0]["evidence"]["proposal"]["priority"] = json!(999);
        assert!(serde_json::from_value::<Response>(invalid.clone()).is_err());
        assert!(!schema("response").is_valid(&invalid));
    }
    let manifest = json!({"id":"fixture","version":"1.0.0","engines":{"reswho":"^0.1.0"},"protocol_version":1,"platforms":["macos"],"capabilities":["classify"],"executable":"/tmp/native","required_features":["tags"],"tag_contract":{"produces":["role"],"overrides":[{"dimension":"role","sources":["other"]}]}});
    assert!(schema("manifest").is_valid(&manifest));
    let mut parsed: Manifest = serde_json::from_value(manifest).unwrap();
    parsed.validate().unwrap();
    parsed.tag_contract.overrides[0].sources.clear();
    assert!(parsed.validate().is_err());
}
