use super::*;
use crate::settings::Config;
use serde_json::{Value, json};
use std::collections::BTreeMap;

fn package(root: &Path, id: &str, version: &str) -> std::path::PathBuf {
    let directory = root.join("extensions").join(id);
    fs::create_dir_all(&directory).unwrap();
    fs::copy("/usr/bin/false", directory.join("provider")).unwrap();
    let manifest = directory.join("manifest.json");
    fs::write(&manifest, json!({"id":id,"version":version,"engines":{"reswho":"*"},"protocol_version":1,"platforms":["macos"],"capabilities":["classify"],"executable":"provider","definitions":[{"key":"shared.order","type":"number","default":1,"description":"Order","sensitive":false}]}).to_string()).unwrap();
    manifest
}

#[test]
fn deterministic_dropin_updates_disable_missing_packages_and_preserve_tombstones() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("extensions.json");
    package(dir.path(), "zeta", "1.0.0");
    let alpha = package(dir.path(), "alpha", "1.0.0");
    let mut state = State::startup(&path).unwrap();
    assert_eq!(
        state
            .installed
            .iter()
            .map(|entry| entry.manifest.id.as_str())
            .collect::<Vec<_>>(),
        ["alpha", "zeta"]
    );
    assert_eq!(state.shared_definitions[0].provider, "alpha");
    let before = fs::read(&path).unwrap();
    State::startup(&path).unwrap();
    assert_eq!(before, fs::read(&path).unwrap());
    State::mutate(&path, |state| state.toggle("alpha", false, false)).unwrap();
    package(dir.path(), "alpha", "2.0.0");
    state = State::startup(&path).unwrap();
    assert!(!state.installed[0].enabled);
    assert_eq!(state.installed[0].manifest.version, "2.0.0");
    assert_eq!(state.shared_definitions[0].provider, "alpha");
    fs::remove_dir_all(alpha.parent().unwrap()).unwrap();
    state = State::startup(&path).unwrap();
    assert!(state.installed[0].problem.is_some());
    package(dir.path(), "alpha", "3.0.0");
    State::mutate(&path, |state| state.toggle("alpha", false, true)).unwrap();
    state = State::startup(&path).unwrap();
    assert!(state.installed[0].removed);
    assert_eq!(state.installed[0].manifest.version, "2.0.0");
    State::mutate(&path, |state| {
        state.install_explicit(Manifest::read(&alpha)?, &alpha)
    })
    .unwrap();
    state = State::startup(&path).unwrap();
    assert!(!state.installed[0].removed);
    assert_eq!(state.installed[0].manifest.version, "3.0.0");
    fs::write(&alpha, "{bad SECRET_SENTINEL").unwrap();
    state = State::startup(&path).unwrap();
    assert!(state.installed[0].problem.is_some());
    let settings = state
        .registry_reported()
        .unwrap()
        .resolve(&Config::default(), &BTreeMap::new())
        .unwrap();
    let active = super::super::external::installed(&state, &settings);
    assert_eq!(active.len(), 1);
    assert_eq!(active[0].manifest.id, "zeta");
    assert!(
        !serde_json::to_string(&state.report)
            .unwrap()
            .contains("SECRET_SENTINEL")
    );
    dir.close().unwrap();
}

#[test]
fn bad_candidates_legacy_ownership_and_nonregular_files_are_isolated() {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("extensions.json");
    let legacy = package(dir.path(), "legacy", "1.0.0");
    State::mutate(&path, |state| state.install(Manifest::read(&legacy)?)).unwrap();
    package(dir.path(), "healthy", "1.0.0");
    package(dir.path(), "docker", "1.0.0");
    let script = package(dir.path(), "script", "1.0.0");
    fs::write(
        script.parent().unwrap().join("provider"),
        "#!/bin/sh\necho invalid native payload\n",
    )
    .unwrap();
    let wrong = package(dir.path(), "wrong", "1.0.0");
    let mut value: Value = serde_json::from_slice(&fs::read(&wrong).unwrap()).unwrap();
    value["id"] = "different".into();
    fs::write(&wrong, value.to_string()).unwrap();
    let fifo = package(dir.path(), "fifo", "1.0.0");
    fs::remove_file(&fifo).unwrap();
    let name = CString::new(fifo.as_os_str().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
    let start = Instant::now();
    let mut state = State::startup(&path).unwrap();
    assert!(start.elapsed().as_secs_f64() < 1.0);
    assert_eq!(state.installed.len(), 2);
    assert!(matches!(state.installed[0].origin, Origin::Legacy));
    assert!(state.installed[0].problem.is_none());
    let settings = state
        .registry_reported()
        .unwrap()
        .resolve(&Config::default(), &BTreeMap::new())
        .unwrap();
    assert_eq!(
        super::super::external::installed(&state, &settings).len(),
        2
    );
    assert!(
        state
            .report
            .iter()
            .any(|record| record.reason.as_deref() == Some("ownership_conflict"))
    );
    assert!(state.report.iter().filter(|record| record.issue()).count() >= 4);
    assert!(state.toggle("docker", false, true).is_err());
    let parsed = Manifest::read(&legacy).unwrap();
    state.install_explicit(parsed.clone(), &legacy).unwrap();
    let other = dir.path().join("other.json");
    fs::write(&other, "{}").unwrap();
    assert!(state.install_explicit(parsed.clone(), &other).is_err());
    state.toggle("legacy", false, true).unwrap();
    state.install_explicit(parsed, &other).unwrap();
    assert!(matches!(state.installed[0].origin, Origin::Explicit { .. }));
    dir.close().unwrap();
}

#[test]
fn persistence_failure_disables_new_candidates_and_retains_healthy_existing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("extensions.json");
    package(dir.path(), "healthy", "1.0.0");
    State::startup(&path).unwrap();
    let before = fs::read(&path).unwrap();
    package(dir.path(), "new", "1.0.0");
    let blocked = path.with_extension(format!("{}.tmp", std::process::id()));
    fs::create_dir(&blocked).unwrap();
    let mut state = State::startup(&path).unwrap();
    let settings = state
        .registry_reported()
        .unwrap()
        .resolve(&Config::default(), &BTreeMap::new())
        .unwrap();
    let providers = super::super::external::installed(&state, &settings);
    assert_eq!(providers.len(), 1);
    assert_eq!(providers[0].manifest.id, "healthy");
    assert_eq!(
        state.installed[1].problem.as_deref(),
        Some("persistence_failed")
    );
    assert_eq!(before, fs::read(&path).unwrap());
    dir.close().unwrap();
}

#[test]
fn concurrent_saved_disable_and_remove_override_stale_discovery() {
    use std::sync::mpsc;
    for remove in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("extensions.json");
        package(dir.path(), "changed", "1.0.0");
        package(dir.path(), "healthy", "1.0.0");
        State::startup(&path).unwrap();
        let stale = State::load(&path).unwrap();
        package(dir.path(), "new", "1.0.0");
        let (locked, acquired) = mpsc::channel();
        let (release, proceed) = mpsc::channel();
        std::thread::scope(|scope| {
            let path = &path;
            let writer = scope.spawn(move || {
                State::mutate(path, |state| {
                    state.toggle("changed", false, remove)?;
                    locked.send(()).unwrap();
                    proceed.recv().unwrap();
                    Ok(())
                })
            });
            acquired.recv().unwrap();
            let (ready, started) = mpsc::channel();
            let reader = scope.spawn(move || {
                ready.send(()).unwrap();
                State::reconcile_startup(stale, path)
            });
            started.recv().unwrap();
            release.send(()).unwrap();
            writer.join().unwrap().unwrap();
            let mut current = reader.join().unwrap().unwrap();
            let settings = current
                .registry_reported()
                .unwrap()
                .resolve(&Config::default(), &BTreeMap::new())
                .unwrap();
            let active = super::super::external::installed(&current, &settings);
            assert_eq!(
                active
                    .iter()
                    .map(|entry| entry.manifest.id.as_str())
                    .collect::<Vec<_>>(),
                ["healthy"]
            );
            let changed = current
                .installed
                .iter()
                .find(|entry| entry.manifest.id == "changed")
                .unwrap();
            assert!(!changed.enabled);
            assert_eq!(changed.removed, remove);
            assert_eq!(
                current
                    .installed
                    .iter()
                    .find(|entry| entry.manifest.id == "new")
                    .unwrap()
                    .problem
                    .as_deref(),
                Some("persistence_failed")
            );
            let saved = State::load(path).unwrap();
            assert_eq!(saved.installed.len(), 2);
            assert_eq!(
                serde_json::to_value(&current.shared_definitions).unwrap(),
                serde_json::to_value(&saved.shared_definitions).unwrap()
            );
        });
        dir.close().unwrap();
    }
}

#[test]
fn newly_reserved_dropin_cannot_clear_persisted_collision() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("extensions.json");
    package(dir.path(), "former", "1.0.0");
    package(dir.path(), "healthy", "1.0.0");
    State::startup(&path).unwrap();
    fs::rename(
        dir.path().join("extensions/former"),
        dir.path().join("extensions/podman"),
    )
    .unwrap();
    let manifest = dir.path().join("extensions/podman/manifest.json");
    let mut package: Value = serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    package["id"] = json!("podman");
    fs::write(&manifest, package.to_string()).unwrap();
    let mut saved: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    saved["installed"][0]["manifest"]["id"] = json!("podman");
    saved["installed"][0]["manifest"]["executable"] =
        json!(dir.path().join("extensions/podman/provider"));
    saved["installed"][0]["manifest"]["definitions"][0]["provider"] = json!("podman");
    saved["installed"][0]["origin"]["manifest"] = json!(manifest);
    saved["shared_definitions"][0]["provider"] = json!("podman");
    fs::write(&path, saved.to_string()).unwrap();
    let before = fs::read(&path).unwrap();
    for package_present in [true, false, true] {
        if package_present {
            fs::write(&manifest, package.to_string()).unwrap();
        } else {
            fs::remove_file(&manifest).unwrap();
        }
        let mut state = State::startup(&path).unwrap();
        assert_eq!(
            state.installed[0].problem.as_deref(),
            Some("external_builtin_id_conflict")
        );
        assert_eq!(fs::read(&path).unwrap(), before);
        let settings = state
            .registry_reported()
            .unwrap()
            .resolve(&Config::default(), &BTreeMap::new())
            .unwrap();
        let active = super::super::external::installed(&state, &settings);
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].manifest.id, "healthy");
        assert_eq!(
            state
                .list(&settings, true)
                .iter()
                .filter(|e| e.descriptor.id == "podman")
                .count(),
            1
        );
    }
    dir.close().unwrap();
}
