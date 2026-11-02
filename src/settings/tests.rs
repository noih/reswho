use super::*;
use serde_json::json;

fn definition(id: &str, key: &str, default: u64) -> Definition {
    Definition {
        key: key.into(),
        contract: Contract {
            value_type: ValueType::Integer,
            unit: Some("ms".into()),
            limits: Limits {
                min: Some(1),
                max: Some(5000),
                allowed: None,
            },
        },
        default: json!(default),
        description: "Fixture setting".into(),
        provider: id.into(),
        version: "1".into(),
        sensitive: false,
    }
}

fn config(values: Value) -> Config {
    serde_json::from_value(json!({"version":1,"settings":values})).unwrap()
}

#[test]
fn ordered_registration_preserves_first_definition_and_resolved_snapshot() {
    let mut registry = Registry::core();
    registry
        .register_extension("a", vec![definition("a", "shared.test", 100)], vec![])
        .unwrap();
    let before = registry
        .resolve(&Config::default(), &BTreeMap::new())
        .unwrap();
    registry
        .register_extension("b", vec![definition("b", "shared.test", 200)], vec![])
        .unwrap();
    let result = registry
        .resolve(&Config::default(), &BTreeMap::new())
        .unwrap();
    assert_eq!(result.get("shared.test"), Some(&json!(100)));
    assert_eq!(
        result.report().settings["shared.test"].definition.provider,
        "a"
    );
    let user = config(json!({"shared.test":300}));
    assert_eq!(
        registry
            .resolve(&user, &BTreeMap::new())
            .unwrap()
            .get("shared.test"),
        Some(&json!(300))
    );
    assert_eq!(before.get("shared.test"), Some(&json!(100)));
    assert_eq!(
        result.interval("sampling.interval_ms"),
        Duration::from_millis(250)
    );
    assert_eq!(
        result.interval("refresh.interval_ms"),
        Duration::from_millis(2000)
    );
}

#[test]
fn conflicting_registration_rolls_back_all_new_keys() {
    for change in 0..4 {
        let mut registry = Registry::core();
        registry
            .register_extension("a", vec![definition("a", "shared.test", 100)], vec![])
            .unwrap();
        let mut conflict = definition("b", "shared.test", 200);
        match change {
            0 => {
                conflict.contract.value_type = ValueType::Boolean;
                conflict.contract.limits = Limits::default();
                conflict.default = json!(true);
            }
            1 => conflict.contract.unit = Some("seconds".into()),
            2 => conflict.contract.limits.max = Some(6000),
            _ => conflict.sensitive = true,
        }
        assert!(
            registry
                .register_extension(
                    "b",
                    vec![definition("b", "shared.new", 1), conflict],
                    vec![]
                )
                .is_err()
        );
        let result = registry
            .resolve(&Config::default(), &BTreeMap::new())
            .unwrap();
        assert!(result.get("shared.new").is_none());
        assert_eq!(result.get("shared.test"), Some(&json!(100)));
        registry.register_extension("b", vec![], vec![]).unwrap();
    }
}

#[test]
fn uses_resolve_after_all_definitions_and_validate_namespaces() {
    let mut registry = Registry::core();
    let usage = Use {
        key: "shared.later".into(),
        contract: definition("b", "shared.later", 1).contract,
    };
    registry
        .register_extension("a", vec![], vec![usage.clone()])
        .unwrap();
    assert!(registry.resolve_uses()["a"][0].contains("missing"));
    registry
        .register_extension("b", vec![definition("b", "shared.later", 1)], vec![])
        .unwrap();
    assert!(registry.resolve_uses().is_empty());
    let mut incompatible = usage;
    incompatible.contract.unit = None;
    registry
        .register_extension("c", vec![], vec![incompatible])
        .unwrap();
    assert!(registry.resolve_uses()["c"][0].contains("incompatible"));
    for key in [
        "other",
        "shared.",
        "shared..bad",
        "extensions.b.private",
        "extensions.a.",
    ] {
        assert!(
            registry
                .register_extension("d", vec![definition("d", key, 1)], vec![])
                .is_err()
        );
    }
    assert!(
        registry
            .register_extension(
                "d",
                vec![definition("d", "shared.new", 1)],
                vec![Use {
                    key: "extensions.a.secret".into(),
                    contract: definition("a", "unused", 1).contract
                }]
            )
            .is_err()
    );
    assert!(
        registry
            .resolve(&Config::default(), &BTreeMap::new())
            .unwrap()
            .get("shared.new")
            .is_none()
    );
    registry
        .register_extension(
            "d",
            vec![definition("d", "extensions.d.private", 1)],
            vec![],
        )
        .unwrap();
}

#[test]
fn invalid_values_never_fall_back_but_cli_can_replace_invalid_user_value() {
    let registry = Registry::core();
    for value in [
        json!(null),
        json!(true),
        json!("250"),
        json!(1.5),
        json!(-1),
        json!(199),
        json!(5001),
    ] {
        let user = config(json!({"sampling.interval_ms":value}));
        assert!(registry.resolve(&user, &BTreeMap::new()).is_err());
        let cli = BTreeMap::from([("sampling.interval_ms".into(), json!(500))]);
        assert_eq!(
            registry
                .resolve(&user, &cli)
                .unwrap()
                .get("sampling.interval_ms"),
            Some(&json!(500))
        );
    }
    for value in [
        json!(0),
        json!(-1),
        json!("2000"),
        json!(1.5),
        json!(18446744073709551616.0),
    ] {
        assert!(
            registry
                .resolve(
                    &config(json!({"refresh.interval_ms":value})),
                    &BTreeMap::new()
                )
                .is_err()
        );
    }
    for value in [200, 5000] {
        assert!(
            registry
                .resolve(
                    &config(json!({"sampling.interval_ms":value})),
                    &BTreeMap::new()
                )
                .is_ok()
        );
    }
    assert!(
        registry
            .resolve(
                &config(json!({"sampling.interval_ms":300})),
                &BTreeMap::from([("sampling.interval_ms".into(), json!(0))])
            )
            .is_err()
    );
}

#[test]
fn exclusive_errors_are_isolated_and_sensitive_reports_are_redacted() {
    let mut registry = Registry::core();
    let mut secret = definition("a", "extensions.a.token", 1);
    secret.contract = Contract {
        value_type: ValueType::String,
        unit: None,
        limits: Limits {
            allowed: Some(vec![json!("SECRET")]),
            ..Limits::default()
        },
    };
    secret.default = json!("SECRET");
    secret.sensitive = true;
    registry
        .register_extension("a", vec![secret], vec![])
        .unwrap();
    let result = registry
        .resolve(&config(json!({"unknown.token":"SECRET"})), &BTreeMap::new())
        .unwrap();
    assert_eq!(result.get("extensions.a.token"), Some(&json!("SECRET")));
    assert!(
        !serde_json::to_string(result.report())
            .unwrap()
            .contains("SECRET")
    );
    assert_eq!(result.report().unknown_keys, vec!["unknown.token"]);
    let result = registry
        .resolve(&config(json!({"extensions.a.token":123})), &BTreeMap::new())
        .unwrap();
    assert!(result.get("extensions.a.token").is_none());
    assert!(result.get("sampling.interval_ms").is_some());
    assert!(result.report().disabled_extensions.contains_key("a"));
    assert!(
        !serde_json::to_string(result.report())
            .unwrap()
            .contains("SECRET")
    );
}

#[test]
fn config_is_versioned_read_only_and_missing_defaults_are_optional() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    assert!(
        Config::load_path(Some(&path), false)
            .unwrap()
            .settings
            .is_empty()
    );
    assert!(Config::load_path(Some(&path), true).is_err());
    for contents in [
        r#"{"version":2,"settings":{}}"#,
        r#"{"version":1,"settings":[]}"#,
        r#"{"version":1,"settings":{"x":"SECRET"},"SECRET":1}"#,
        "SECRET",
        r#"{"settings":{}}"#,
    ] {
        std::fs::write(&path, contents).unwrap();
        let error = Config::load_path(Some(&path), true)
            .err()
            .unwrap()
            .to_string();
        assert!(!error.contains("SECRET"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), contents);
    }
    std::fs::write(
        &path,
        r#"{"version":1,"settings":{"sampling.interval_ms":450}}"#,
    )
    .unwrap();
    assert_eq!(
        Config::load_path(Some(&path), true).unwrap().settings["sampling.interval_ms"],
        450
    );
    assert_eq!(
        path_from_env(Some("/tmp/xdg".into()), Some("/tmp/home".into())),
        Some(PathBuf::from("/tmp/xdg/reswho/config.json"))
    );
    assert_eq!(
        path_from_env(None, Some("/tmp/home".into())),
        Some(PathBuf::from("/tmp/home/.config/reswho/config.json"))
    );
    assert_eq!(path_from_env(None, None), None);
}

#[test]
fn defaults_types_allowed_values_and_owners_are_validated_before_commit() {
    let mut registry = Registry::core();
    for default in [json!(0), json!(5001), json!(true), json!("SECRET")] {
        let mut invalid = definition("a", "shared.bad", 1);
        invalid.default = default;
        let error = registry
            .register_extension(
                "a",
                vec![definition("a", "shared.first", 1), invalid],
                vec![],
            )
            .err()
            .unwrap()
            .to_string();
        assert!(!error.contains("SECRET"));
        assert!(
            registry
                .resolve(&Config::default(), &BTreeMap::new())
                .unwrap()
                .get("shared.first")
                .is_none()
        );
    }
    let mut boolean = definition("a", "extensions.a.enabled", 1);
    boolean.contract = Contract {
        value_type: ValueType::Boolean,
        unit: None,
        limits: Limits::default(),
    };
    boolean.default = json!(true);
    registry
        .register_extension("a", vec![boolean], vec![])
        .unwrap();
    let valid = registry
        .resolve(
            &config(json!({"extensions.a.enabled":false})),
            &BTreeMap::new(),
        )
        .unwrap();
    assert_eq!(valid.get("extensions.a.enabled"), Some(&json!(false)));
    let invalid = registry
        .resolve(
            &config(json!({"extensions.a.enabled":"false"})),
            &BTreeMap::new(),
        )
        .unwrap();
    assert!(invalid.report().disabled_extensions.contains_key("a"));
    let mut choice = definition("b", "shared.choice", 1);
    choice.contract.limits.allowed = Some(vec![json!(1), json!(2)]);
    registry
        .register_extension("b", vec![choice], vec![])
        .unwrap();
    assert!(
        registry
            .resolve(&config(json!({"shared.choice":3})), &BTreeMap::new())
            .is_err()
    );
    let mut impostor = definition("a", "shared.impostor", 1);
    impostor.provider = "b".into();
    assert!(
        registry
            .register_extension("c", vec![impostor], vec![])
            .is_err()
    );
    assert!(registry.register_extension("a", vec![], vec![]).is_err());
    assert!(registry.register_extension("core", vec![], vec![]).is_err());
}

#[test]
fn allowed_sets_ignore_order_in_declarations_and_uses_without_changing_canonical_definition() {
    let mut registry = Registry::core();
    let mut first = definition("a", "shared.choice", 1);
    first.contract.limits.allowed = Some(vec![json!(1), json!(2)]);
    registry
        .register_extension("a", vec![first.clone()], vec![])
        .unwrap();
    for (id, allowed) in [
        ("b", vec![json!(2), json!(1)]),
        ("c", vec![json!(2), json!(1), json!(2)]),
    ] {
        let mut candidate = definition(id, "shared.choice", 2);
        candidate.contract.limits.allowed = Some(allowed);
        let usage = Use {
            key: candidate.key.clone(),
            contract: candidate.contract.clone(),
        };
        registry
            .register_extension(id, vec![candidate], vec![usage])
            .unwrap();
    }
    assert!(registry.resolve_uses().is_empty());
    let result = registry
        .resolve(&Config::default(), &BTreeMap::new())
        .unwrap();
    let canonical = &result.report().settings["shared.choice"].definition;
    assert_eq!(canonical.provider, "a");
    assert_eq!(canonical.default, json!(1));
    assert_eq!(
        canonical.contract.limits.allowed,
        first.contract.limits.allowed
    );
    for allowed in [
        vec![json!(1)],
        vec![json!(1), json!(2), json!(3)],
        vec![json!(1), json!(3)],
    ] {
        let mut conflicting = definition("d", "shared.choice", 1);
        conflicting.contract.limits.allowed = Some(allowed);
        let usage = Use {
            key: conflicting.key.clone(),
            contract: conflicting.contract.clone(),
        };
        assert!(
            registry
                .register_extension(
                    "d",
                    vec![definition("d", "shared.rollback", 1), conflicting],
                    vec![]
                )
                .is_err()
        );
        assert!(
            registry
                .resolve(&Config::default(), &BTreeMap::new())
                .unwrap()
                .get("shared.rollback")
                .is_none()
        );
        let mut uses_registry = Registry::core();
        uses_registry
            .register_extension("a", vec![first.clone()], vec![])
            .unwrap();
        uses_registry
            .register_extension("d", vec![], vec![usage])
            .unwrap();
        assert!(uses_registry.resolve_uses()["d"][0].contains("incompatible use"));
    }
}
