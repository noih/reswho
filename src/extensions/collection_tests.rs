use super::*;
use crate::extensions::tests::Fixture;
use serde_json::{Value, json};

fn collection_fixture(provider: &'static str, contribution: Value) -> Fixture {
    Fixture::new(provider, serde_json::from_value(contribution).unwrap())
}

fn container_contribution(provider: &str) -> serde_json::Value {
    let mut contribution: serde_json::Value = serde_json::from_str(include_str!(
        "../../tests/fixtures/internal-containers.json"
    ))
    .unwrap();
    contribution["runtime"]["name"] = json!(provider);
    contribution["containers"][0]["runtime"] = json!(provider);
    contribution["containers"][0]["id"] = json!(provider);
    contribution["containers"][0]["cpu_percent"] = json!(125.5);
    contribution["runtime"]["unavailable"] = json!([{"code":"collection_note","operation":"extensions.collect","message":"Partial collection note","os_error":null}]);
    contribution
}

fn assert_collection_isolation(cases: &[&str]) {
    let good = collection_fixture("good", container_contribution("good"));
    let Contribution::Collect {
        runtime,
        containers,
    } = serde_json::from_value(container_contribution("good")).unwrap()
    else {
        unreachable!()
    };
    let expected_healthy = crate::containers::build_all(vec![(runtime, containers)])
        .unwrap()
        .children
        .remove(0);

    let resources = collection_fixture(
        "independent",
        json!({"capability":"collected","runtime":null,"containers":[],"resources":[{"provider":"independent","id":"volume","kind":"volume","label":"Independent volume","path":null,"metrics":[]}]}),
    );
    for case in cases {
        let mut contribution = container_contribution("bad");
        if *case != "runtime_namespace" {
            contribution["capability"] = json!("collected");
            contribution["resources"] = json!([]);
        }
        match *case {
            "runtime_namespace" | "collected_runtime_namespace" => {
                contribution["runtime"]["name"] = json!("good");
                contribution["containers"][0]["runtime"] = json!("good");
            }
            "container_runtime" => contribution["containers"][0]["runtime"] = json!("good"),
            "container_endpoint" => {
                contribution["containers"][0]["endpoint_ref"] = json!("another-endpoint")
            }
            "missing_endpoint" => contribution["runtime"]["endpoint_ref"] = serde_json::Value::Null,
            "missing_runtime" => {
                contribution["capability"] = json!("collected");
                contribution["runtime"] = serde_json::Value::Null;
                contribution["resources"] = json!([]);
            }
            "status" => contribution["runtime"]["status"] = json!("invented"),
            "empty_id" => contribution["containers"][0]["id"] = json!(""),
            "diagnostic" => contribution["runtime"]["unavailable"][0]["code"] = json!("invented"),
            "duplicate_id" => {
                let container = contribution["containers"][0].clone();
                contribution["containers"]
                    .as_array_mut()
                    .unwrap()
                    .push(container);
            }
            "negative_cpu" => contribution["containers"][0]["cpu_percent"] = json!(-1),
            "inactive_metrics" => contribution["containers"][0]["state"] = json!("exited"),
            "cpu_overflow" => {
                contribution["containers"][0]["cpu_percent"] = json!(1e308);
                let mut container = contribution["containers"][0].clone();
                container["id"] = json!("other");
                contribution["containers"]
                    .as_array_mut()
                    .unwrap()
                    .push(container);
            }
            "metrics_scope" => {
                contribution["containers"][0]["metrics_scope"] = json!("x".repeat(65))
            }
            "configuration_scope" => contribution["runtime"]["configuration"] = json!({"scope":""}),
            _ => unreachable!(),
        }
        let bad = collection_fixture("bad", contribution);
        let settings = super::default_settings().unwrap();
        let cancel = Cancellation::default();
        let context = Context::new(&settings, &cancel);
        let providers: Vec<&dyn Extension> = vec![&good, &bad, &resources];
        let (containers, resource_root) = super::collect_scoped(&providers, &context, true)
            .unwrap_or_else(|error| panic!("{case}: healthy collection lost: {error}"));
        let totals = containers.container_totals.as_ref().unwrap();
        assert_eq!(totals.containers, 1, "{case}");
        assert_eq!(totals.known_cpu_percent, 125.5, "{case}");
        assert_eq!(totals.known_memory_bytes, 4096, "{case}");
        assert_eq!(
            resource_root.children[0].children[0].name, "Independent volume",
            "{case}"
        );
        let healthy = containers
            .children
            .iter()
            .find(|node| node.runtime.as_ref().is_some_and(|r| r.name == "good"))
            .unwrap();
        assert_eq!(
            serde_json::to_value(healthy).unwrap(),
            serde_json::to_value(&expected_healthy).unwrap(),
            "{case}: healthy refs and data changed"
        );
        assert_eq!(
            healthy.children[0].children[0]
                .container
                .as_ref()
                .unwrap()
                .id,
            "good"
        );
        let records = context.report.lock().unwrap().clone();
        assert!(
            !records
                .iter()
                .any(|record| record.provider == "bad" && record.result == "success"),
            "{case}: rejected source reported success"
        );
        for (id, result) in [
            ("good", "success"),
            ("bad", "failed"),
            ("independent", "success"),
        ] {
            assert!(
                records
                    .iter()
                    .any(|r| r.provider == id && r.phase == "collect" && r.result == result),
                "{case}: {id} must be {result}"
            );
        }
        let mut root = crate::model::Node::group(
            "group:all".into(),
            "Resources".into(),
            vec![],
            crate::model::Grouping::Category,
        );
        crate::tree::compose_category(&mut root, containers).unwrap();
        crate::tree::compose_category(&mut root, resource_root).unwrap();
        crate::tree::validate_references(&root).unwrap();
        let snapshot = crate::model::Snapshot {
            schema: crate::model::JSON_SCHEMA_VERSION,
            extensions: records,
            captured_unix_ms: 0,
            sample_ms: 250,
            collection_ms: 1,
            system: Default::default(),
            scope: "group:all".into(),
            root,
            warnings: vec![],
        };
        let schema: serde_json::Value = serde_json::from_str(crate::output::JSON_SCHEMA).unwrap();
        assert!(
            jsonschema::validator_for(&schema)
                .unwrap()
                .is_valid(&serde_json::to_value(snapshot).unwrap()),
            "{case}: invalid schema"
        );
        drop(providers);
        drop(context);
        drop(settings);
    }
}

#[test]
fn review_r1_internal_collect_identity_failure_preserves_healthy_siblings() {
    assert_collection_isolation(&[
        "runtime_namespace",
        "collected_runtime_namespace",
        "container_runtime",
        "container_endpoint",
        "missing_endpoint",
        "missing_runtime",
        "status",
        "empty_id",
        "duplicate_id",
        "diagnostic",
    ]);
}

#[test]
fn review_r1_internal_collect_metrics_failure_preserves_healthy_siblings() {
    assert_collection_isolation(&[
        "negative_cpu",
        "inactive_metrics",
        "cpu_overflow",
        "metrics_scope",
        "configuration_scope",
    ]);
}
