use super::*;
use crate::model::Snapshot;
use serde_json::{Value, json};
use std::fs;

#[derive(Clone)]
struct Fixture {
    id: String,
    registration: placement::Registration,
    contribution: Contribution,
}
impl Fixture {
    fn new(id: &str, response: &Value) -> Self {
        Self {
            id: id.into(),
            registration: Default::default(),
            contribution: serde_json::from_value(response["contribution"].clone()).unwrap(),
        }
    }
}
impl Extension for Fixture {
    fn tag_contract(&self) -> Option<tags::Contract> {
        Some(tags::Contract {
            produces: self
                .registration
                .ownership
                .iter()
                .map(|o| o.dimension.clone())
                .collect(),
            overrides: vec![],
        })
    }
    fn descriptor(&self) -> Descriptor {
        Descriptor {
            id: self.id.clone(),
            capabilities: vec![Capability::Classify, Capability::Collect],
            enabled: true,
            availability: "available",
        }
    }
    fn registration(&self) -> placement::Registration {
        self.registration.clone()
    }
    fn execute(&self, _: &Context<'_>, _: Request<'_>) -> Result<Contribution> {
        Ok(self.contribution.clone())
    }
}

fn process(cwd: &Path) -> Process {
    Process {
        pid: 42,
        started_unix_us: Some(123),
        ppid: Some(1),
        uid: Some(501),
        name: "fixture".into(),
        executable: Some("/tmp/fixture".into()),
        cwd: Some(cwd.into()),
        resident_bytes: Some(123),
        cpu_percent: Some(1.0),
        ..Default::default()
    }
}

#[test]
fn internal_placement_registers_categories_and_projection_with_safe_failure_isolation() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().canonicalize().unwrap();
    let projection = json!({"grouping":"unknown","parents":[{"reference":"group:extensions.placement_fixture.parent","name":"Native parent","grouping":"unknown","evidence":[],"attribution":null,"diagnostics":[],"collapse_single":false,"child_name":null}]});
    let tag = json!({"dimension":"extensions.placement_fixture.owner","id":"extensions.placement_fixture.owner","label":"Native owner","path":cwd,"basis":"cwd","projection":projection});
    let proposal = json!({"identity":{"pid":42,"started_unix_us":123},"evidence":{"kind":"tags","proposal":{"id":"owner","identity":{"pid":42,"started_unix_us":123},"change":{"operation":"add","tag":tag}}}});
    let response = json!({"contribution":{"capability":"classify","proposals":[proposal]}});
    let registration = json!({"categories":[{"id":"extensions.placement_fixture.category","label":"Native category","order":50}],"ownership":[{"dimension":"extensions.placement_fixture.owner","category":"extensions.placement_fixture.category","priority":50}]});
    let mut provider = Fixture::new("placement_fixture", &response);
    provider.registration = serde_json::from_value(registration).unwrap();
    let cancel = Cancellation::default();
    let settings = super::default_settings().unwrap();
    let root = crate::tree::build_with_providers(
        vec![process(&cwd)],
        &Context::new(&settings, &cancel),
        &[&provider],
    )
    .unwrap();
    let category = crate::tree::select(
        &root,
        &super::placement::category_ref("extensions.placement_fixture.category"),
    )
    .unwrap();
    assert_eq!(category.name, "Native category");
    assert_eq!(category.children[0].name, "Native parent");
    assert_eq!(category.children[0].children[0].name, "Native owner");
    assert_eq!(root.totals.processes, 1);
    assert_eq!(root.totals.resident_bytes, 123);
    let snapshot = Snapshot {
        schema: crate::model::JSON_SCHEMA_VERSION,
        extensions: vec![],
        captured_unix_ms: 0,
        sample_ms: 250,
        collection_ms: 1,
        system: Default::default(),
        scope: "group:all".into(),
        root,
        warnings: vec![],
    };
    assert!(crate::output::markdown(&snapshot).contains("Native category"));
    crate::tui::tests::assert_extension_category(
        snapshot.clone(),
        "Native category",
        "Native owner",
    );
    let value = serde_json::to_value(&snapshot).unwrap();
    let schema: Value = serde_json::from_str(crate::output::JSON_SCHEMA).unwrap();
    let errors: Vec<_> = jsonschema::validator_for(&schema)
        .unwrap()
        .iter_errors(&value)
        .map(|e| e.to_string())
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
    assert!(!value.to_string().contains("\"projection\""));
    let selected = crate::tree::select(&snapshot.root, "pid:42").unwrap();
    assert!(
        selected
            .process
            .as_ref()
            .unwrap()
            .tag_report
            .as_ref()
            .unwrap()
            .effective
            .iter()
            .any(|entry| entry.tag.projection.is_some())
    );
    let selected_value = serde_json::to_value(review_snapshot(selected, "pid:42")).unwrap();
    assert!(
        jsonschema::validator_for(&schema)
            .unwrap()
            .is_valid(&selected_value)
    );
    assert!(!selected_value.to_string().contains("\"projection\""));
    let nested = cwd.join("nested");
    fs::create_dir(&nested).unwrap();
    let nested = crate::tree::build_with_providers(
        vec![process(&nested)],
        &Context::new(&settings, &cancel),
        &[&provider],
    )
    .unwrap();
    assert!(
        crate::tree::select(
            &nested,
            &super::placement::category_ref("extensions.placement_fixture.category")
        )
        .is_ok()
    );
    let outside = tempfile::tempdir().unwrap();
    let outside_tree = crate::tree::build_with_providers(
        vec![process(&outside.path().canonicalize().unwrap())],
        &Context::new(&settings, &cancel),
        &[&provider],
    )
    .unwrap();
    assert_eq!(outside_tree.totals.processes, 1);
    assert_eq!(outside_tree.diagnostics.len(), 1);
    outside.close().unwrap();

    provider.registration.ownership[0].priority = 900;
    let low = crate::tree::build_with_providers(
        vec![process(&cwd)],
        &Context::new(&settings, &cancel),
        &[&provider],
    )
    .unwrap();
    assert!(
        crate::tree::select(
            &low,
            &super::placement::category_ref("extensions.placement_fixture.category")
        )
        .is_err()
    );
    assert_eq!(low.totals.resident_bytes, 123);
    let mut preferred = super::default_settings().unwrap();
    preferred.ownership_priority = vec!["extensions.placement_fixture.owner".into()];
    let preferred = crate::tree::build_with_providers(
        vec![process(&cwd)],
        &Context::new(&preferred, &cancel),
        &[&provider],
    )
    .unwrap();
    assert!(
        crate::tree::select(
            &preferred,
            &super::placement::category_ref("extensions.placement_fixture.category")
        )
        .is_ok()
    );
    provider.registration.ownership[0].priority = 50;

    let mut bad = provider.clone();
    bad.registration.categories = vec![placement::Category::new("projects", "Collision", 1)];
    bad.registration.ownership[0].category = "projects".into();
    let context = Context::new(&settings, &cancel);
    let fallback =
        crate::tree::build_with_providers(vec![process(&cwd)], &context, &[&bad]).unwrap();
    assert_eq!(fallback.totals.processes, 1);
    assert!(
        crate::tree::select(
            &fallback,
            &placement::category_ref("extensions.placement_fixture.category")
        )
        .is_err()
    );
    assert!(!fallback.diagnostics.is_empty());
    dir.close().unwrap();
}

fn review_owner(_dir: &Path, category: &str, label: &str) -> Fixture {
    let contribution: Value =
        serde_json::from_str(include_str!("../../tests/fixtures/internal-placement.json")).unwrap();
    let mut response = json!({"contribution": contribution});
    response["contribution"]["proposals"][0]["evidence"]["proposal"]["change"]["tag"]["projection"] =
        Value::Null;
    let mut provider = Fixture::new("fixture", &response);
    provider.registration = serde_json::from_value(json!({"categories":[{"id":category,"label":label,"order":150}],"ownership":[{"dimension":"extensions.fixture.owner","category":category,"priority":50}]})).unwrap();
    provider
}

fn review_snapshot(root: crate::model::Node, scope: &str) -> Snapshot {
    Snapshot {
        schema: crate::model::JSON_SCHEMA_VERSION,
        extensions: vec![],
        captured_unix_ms: 0,
        sample_ms: 250,
        collection_ms: 1,
        system: Default::default(),
        scope: scope.into(),
        root,
        warnings: vec![],
    }
}
#[test]
fn review_registration_sensitive_label_is_isolated_from_all_outputs() {
    let dir = tempfile::tempdir().unwrap();
    let secret = "review-known-secret";
    let provider = review_owner(dir.path(), "extensions.fixture.jobs", secret);
    let definitions = vec![serde_json::from_value(json!({"key":"extensions.fixture.token","type":"string","default":secret,"description":"Synthetic token","provider":"fixture","sensitive":true})).unwrap()];
    let mut registry = super::settings_registry().unwrap();
    registry
        .register_extension("fixture", definitions, vec![])
        .unwrap();
    let settings = registry
        .resolve(&Default::default(), &Default::default())
        .unwrap();
    let cancel = Cancellation::default();
    let context = Context::new(&settings, &cancel);
    let root = crate::tree::build_with_providers(vec![process(dir.path())], &context, &[&provider])
        .unwrap();
    let mut snapshot = review_snapshot(root, "group:all");
    snapshot.extensions = context.report.lock().unwrap().clone();
    assert!(!serde_json::to_string(&snapshot).unwrap().contains(secret));
    assert!(!crate::output::markdown(&snapshot).contains(secret));
    assert!(
        !crate::tui::tests::assert_extension_category(snapshot, "Unclassified", "fixture")
            .contains(secret)
    );
    assert!(context.failed.lock().unwrap().contains("fixture"));
    dir.close().unwrap();
}
#[test]
fn review_shared_category_composes_internal_host_and_collector_scopes() {
    let dir = tempfile::tempdir().unwrap();
    let owner = review_owner(dir.path(), "containers", "Containers");
    let settings = super::default_settings().unwrap();
    let cancel = Cancellation::default();
    for empty in [true, false] {
        let contribution: Value = serde_json::from_str(include_str!(
            "../../tests/fixtures/internal-containers.json"
        ))
        .unwrap();
        let mut response = json!({"contribution": contribution});
        response["provider"] = json!("collector");
        response["contribution"]["runtime"]["name"] = json!("collector");
        for container in response["contribution"]["containers"]
            .as_array_mut()
            .unwrap()
        {
            container["runtime"] = json!("collector");
        }
        if empty {
            response["contribution"]["containers"] = json!([]);
        }
        let collector = Fixture::new("collector", &response);
        let context = Context::new(&settings, &cancel);
        let mut root =
            crate::tree::build_with_providers(vec![process(dir.path())], &context, &[&owner])
                .unwrap();
        let (containers, _) = super::collect_scoped(&[&collector], &context, true).unwrap();
        assert!(!containers.children.is_empty());
        assert!(!context.failed.lock().unwrap().contains("collector"));
        assert!(
            containers
                .container_totals
                .as_ref()
                .unwrap()
                .inventory_complete
        );
        let mut collision = containers.clone();
        collision
            .children
            .push(crate::tree::select(&root, "pid:42").unwrap());
        let before = serde_json::to_value(&root).unwrap();
        assert!(crate::tree::compose_category(&mut root, collision).is_err());
        assert_eq!(serde_json::to_value(&root).unwrap(), before);
        crate::tree::compose_category(&mut root, containers.clone()).unwrap();
        crate::tree::validate_references(&root).unwrap();
        let category = crate::tree::select(&root, "group:containers").unwrap();
        assert_eq!(category.totals.processes, 1);
        assert_eq!(category.totals.resident_bytes, 123);
        assert_eq!(
            crate::sort::Sort::Memory
                .compare(&crate::tree::select(&root, "pid:42").unwrap(), &category),
            std::cmp::Ordering::Less
        );
        assert_eq!(
            category.container_totals.as_ref().unwrap().containers,
            usize::from(!empty)
        );
        assert_eq!(root.totals.processes, 1);
        assert_eq!(
            category
                .container_totals
                .as_ref()
                .unwrap()
                .known_memory_bytes,
            if empty { 0 } else { 4096 }
        );
        let mut bad = owner.clone();
        bad.registration.categories[0].label = "Incompatible category".into();
        let isolated = Context::new(&settings, &cancel);
        let mut fallback =
            crate::tree::build_with_providers(vec![process(dir.path())], &isolated, &[&bad])
                .unwrap();
        assert!(isolated.failed.lock().unwrap().contains("fixture"));
        assert!(crate::tree::select(&fallback, "group:containers").is_err());
        crate::tree::compose_category(&mut fallback, containers).unwrap();
        assert_eq!(fallback.totals.processes, 1);
        assert!(crate::tree::select(&fallback, "group:unclassified").is_ok());
        crate::tree::validate_references(&fallback).unwrap();
        let snapshot = review_snapshot(root, "group:all");
        let value = serde_json::to_value(&snapshot).unwrap();
        assert!(
            jsonschema::validator_for(
                &serde_json::from_str::<Value>(crate::output::JSON_SCHEMA).unwrap()
            )
            .unwrap()
            .is_valid(&value)
        );
        let md = crate::output::markdown(&snapshot);
        assert!(
            md.contains("host") && md.contains("containers") && md.contains("Σ1"),
            "{md}"
        );
        let tui =
            crate::tui::tests::assert_extension_category(snapshot, "Containers", "Fixture job");
        let row = tui
            .lines()
            .find(|line| line.contains("Containers") && !line.contains("scope"))
            .unwrap();
        assert!(row.contains("1 proc") && row.contains("ctr"), "{row}");
        assert_eq!(
            row.split_whitespace().filter(|value| *value == "-").count(),
            3,
            "{row}"
        );
    }
    dir.close().unwrap();
}
