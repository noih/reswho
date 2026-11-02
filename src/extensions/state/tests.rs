use super::*;
use serde_json::json;

fn manifest(id: &str) -> Manifest {
    let mut value: Manifest = serde_json::from_value(json!({"id":id,"version":"1.0.0","engines":{"reswho":">=0.1.0 <0.2.0"},"protocol_version":1,"platforms":[std::env::consts::OS],"capabilities":["classify"],"executable":"/usr/bin/false"})).unwrap();
    value.validate().unwrap();
    value
}

#[test]
fn npm_ranges_and_versions_use_strict_engines_semantics() {
    for (version, range, expected) in [
        ("1.2.3", ">=1.2.0 <2", true),
        ("2.0.0", ">=1.2.0 <2", false),
        ("0.2.3", "^0.2.0", true),
        ("0.3.0", "^0.2.0", false),
        ("0.0.4", "^0.0.3", false),
        ("1.3.0", "~1.2.0", false),
        ("2.0.0", "1.x || >=2 <=2.0.0", true),
        ("1.2.3-beta.1", ">=1.0.0", false),
        ("1.2.3-beta.1", ">=1.2.3-beta.0 <2", true),
        ("1.2.0", "1.0.0 - 1.3.0", true),
        ("1.2.0", ">1.2.0", false),
    ] {
        assert_eq!(
            version
                .parse::<Version>()
                .unwrap()
                .satisfies(&range.parse::<Range>().unwrap()),
            expected
        );
    }
    let mut value = manifest("demo");
    assert!(value.incompatibility().is_none());
    value.engines.insert("reswho".into(), ">99".into());
    assert!(value.incompatibility().is_some());
    value
        .engines
        .insert("reswho".into(), "this is not a range".into());
    assert!(value.validate().is_err());
    value.engines.insert("reswho".into(), "*".into());
    value.version = "v1.0.0".into();
    assert!(value.validate().is_err());
}

#[test]
fn lifecycle_keeps_order_canonical_definitions_and_is_atomic() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("extensions.json");
    let mut a = manifest("a");
    a.definitions = vec![serde_json::from_value(json!({"key":"shared.example","type":"number","integer":true,"unit":"ms","min":1,"max":5000,"default":100,"description":"example","sensitive":false})).unwrap()];
    a.validate().unwrap();
    State::mutate(&path, |state| state.install(a.clone())).unwrap();
    let mut b = manifest("b");
    b.definitions = a.definitions.clone();
    b.definitions[0].provider.clear();
    b.definitions[0].default = json!(200);
    b.validate().unwrap();
    State::mutate(&path, |state| state.install(b.clone())).unwrap();
    State::mutate(&path, |state| state.toggle("a", false, true)).unwrap();
    let state = State::load(&path).unwrap();
    assert_eq!(state.shared_definitions[0].provider, "a");
    assert_eq!(state.shared_definitions[0].default, 100);
    State::mutate(&path, |state| state.install(a.clone())).unwrap();
    assert_eq!(
        State::load(&path)
            .unwrap()
            .installed
            .iter()
            .map(|entry| entry.manifest.id.as_str())
            .collect::<Vec<_>>(),
        ["a", "b"]
    );
    let before = fs::read(&path).unwrap();
    b.definitions[0].contract.unit = None;
    assert!(State::mutate(&path, |state| state.install(b)).is_err());
    assert_eq!(fs::read(&path).unwrap(), before);
    fs::write(&path, "CORRUPT").unwrap();
    assert!(State::mutate(&path, |_| Ok(())).is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), "CORRUPT");
    dir.close().unwrap();
}

#[test]
fn forward_uses_and_concurrent_mutations_survive_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("extensions.json");
    let mut early = manifest("early");
    early.uses = vec![
        serde_json::from_value(json!({"key":"shared.later","type":"boolean","unit":null})).unwrap(),
    ];
    State::mutate(&path, |state| state.install(early)).unwrap();
    assert!(
        State::load(&path)
            .unwrap()
            .registry()
            .unwrap()
            .resolve_uses()
            .contains_key("early")
    );
    let mut late = manifest("late");
    late.definitions = vec![serde_json::from_value(json!({"key":"shared.later","type":"boolean","unit":null,"default":true,"description":"later","sensitive":false})).unwrap()];
    late.validate().unwrap();
    State::mutate(&path, |state| state.install(late)).unwrap();
    assert!(
        State::load(&path)
            .unwrap()
            .registry()
            .unwrap()
            .resolve_uses()
            .is_empty()
    );
    std::thread::scope(|scope| {
        for id in ["one", "two"] {
            let path = &path;
            scope.spawn(move || State::mutate(path, |state| state.install(manifest(id))).unwrap());
        }
    });
    assert_eq!(State::load(&path).unwrap().installed.len(), 4);
    assert!(
        !path
            .with_extension(format!("{}.tmp", std::process::id()))
            .exists()
    );
    dir.close().unwrap();
}

#[test]
fn incompatible_installs_do_not_inject_definitions_and_disabled_updates_stay_disabled() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("extensions.json");
    let mut value = manifest("future");
    value.engines.insert("reswho".into(), ">99".into());
    value.definitions=vec![serde_json::from_value(json!({"key":"shared.future","type":"boolean","unit":null,"default":true,"description":"future","sensitive":false})).unwrap()];
    State::mutate(&path, |state| state.install(value.clone())).unwrap();
    let state = State::load(&path).unwrap();
    assert!(state.shared_definitions.is_empty());
    assert!(
        state
            .registry()
            .unwrap()
            .definition("shared.future")
            .is_none()
    );
    value
        .engines
        .insert("reswho".into(), ">=0.1.0 <0.2.0".into());
    State::mutate(&path, |state| state.install(value.clone())).unwrap();
    assert!(
        State::load(&path)
            .unwrap()
            .registry()
            .unwrap()
            .definition("shared.future")
            .is_some()
    );
    State::mutate(&path, |state| state.toggle("future", false, false)).unwrap();
    State::mutate(&path, |state| state.install(value)).unwrap();
    assert!(!State::load(&path).unwrap().installed[0].enabled);
    dir.close().unwrap();
}

#[test]
fn native_packaging_rejects_script_args_and_disables_replaced_binary() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let binary = dir.path().join("provider");
    fs::copy(std::env::current_exe().unwrap(), &binary).unwrap();
    let mut value = manifest("native");
    value.executable = binary.clone();
    let mut state = State::default();
    state.install(value.clone()).unwrap();
    value.args = vec!["payload.py".into()];
    assert!(state.install(value).is_err());
    fs::write(&binary, "#!/bin/sh\nexit 0\n").unwrap();
    fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
    let settings = state
        .registry()
        .unwrap()
        .resolve(&crate::settings::Config::default(), &BTreeMap::new())
        .unwrap();
    let list = state.list(&settings, true);
    let entry = list
        .iter()
        .find(|entry| entry.descriptor.id == "native")
        .unwrap();
    assert!(!entry.descriptor.enabled);
    assert!(entry.explicit_enabled);
    assert!(!entry.compatible);
    assert!(super::super::external::installed(&state, &settings).is_empty());
    assert!(state.install(state.installed[0].manifest.clone()).is_err());
    dir.close().unwrap();
}
