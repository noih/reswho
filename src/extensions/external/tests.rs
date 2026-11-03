use super::*;
use crate::model::{Snapshot, SystemSummary};
use std::fs;

fn fixture(dir: &Path, id: &str, body: &str) -> External {
    let executable = dir.join(id);
    let source = dir.join(format!("{id}.c"));
    fs::write(&source, format!("#include <stdio.h>\n#include <stdlib.h>\n#include <unistd.h>\n#include <signal.h>\n#include <string.h>\n#include <assert.h>\nint main(void) {{ {body}; return 0; }}")).unwrap();
    assert!(
        std::process::Command::new("cc")
            .arg(&source)
            .arg("-o")
            .arg(&executable)
            .status()
            .unwrap()
            .success()
    );
    let mut manifest: Manifest = serde_json::from_value(json!({"id":id,"version":"1.0.0","engines":{"reswho":">=0.1.0 <0.2"},"protocol_version":1,"platforms":[std::env::consts::OS],"capabilities":["collect","classify","details"],"collect_scopes":["containers"],"executable":executable})).unwrap();
    manifest.validate().unwrap();
    External { manifest }
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
fn installed_example_contributes_custom_classification_details_and_shared_settings() {
    let dir = tempfile::tempdir().unwrap();
    let state_path = dir.path().join("extensions.json");
    fs::write(dir.path().join(".reswho-example"), "").unwrap();
    let manifest = Manifest::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("examples/extensions/example.json")
            .as_path(),
    )
    .unwrap();
    State::mutate(&state_path, |state| state.install(manifest)).unwrap();
    let state = State::load(&state_path).unwrap();
    let settings = state
        .registry()
        .unwrap()
        .resolve(
            &Config::default(),
            &BTreeMap::from([("sampling.interval_ms".into(), json!(450))]),
        )
        .unwrap();
    let providers = installed(&state, &settings);
    let cancel = Cancellation::default();
    let context = Context::new(&settings, &cancel);
    let mut process = process(dir.path());
    assert!(process.command.is_none());
    let errors = attach_details(&[&providers[0]], &context, &mut process);
    assert!(errors.is_empty());
    assert_eq!(process.extension_details[0].fields[0].value, 450);
    assert_eq!(process.extension_details[0].fields[1].value, HOST_VERSION);
    assert_eq!(process.extension_details[0].fields[2].value, "[filtered]");
    let root = crate::tree::build_with_providers(
        vec![process.clone()],
        &context,
        &[&Builtin::AppBundle, &providers[0]],
    )
    .unwrap();
    assert_eq!(root.totals.processes, 1);
    let full = Snapshot {
        schema: crate::model::JSON_SCHEMA_VERSION,
        extensions: vec![],
        captured_unix_ms: 0,
        sample_ms: 450,
        collection_ms: 450,
        system: SystemSummary::default(),
        scope: "group:all".into(),
        root: root.clone(),
        warnings: vec![],
    };
    let schema: Value = serde_json::from_str(crate::output::JSON_SCHEMA).unwrap();
    let full_json = serde_json::to_value(full).unwrap();
    let errors: Vec<_> = jsonschema::validator_for(&schema)
        .unwrap()
        .iter_errors(&full_json)
        .map(|error| error.to_string())
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
    let mut app_process = process.clone();
    app_process.executable = Some("/Applications/Demo.app/Contents/MacOS/demo".into());
    let app_root = crate::tree::build_with_providers(
        vec![app_process],
        &context,
        &[&Builtin::AppBundle, &providers[0]],
    )
    .unwrap();
    assert!(matches!(
        crate::tree::select(&app_root, "pid:42")
            .unwrap()
            .attribution
            .unwrap()
            .source,
        crate::model::AttributionSource::AppBundle
    ));
    fs::write(dir.path().join("go.mod"), "module fixture").unwrap();
    let project_root = crate::tree::build_with_providers(
        vec![process],
        &context,
        &[&Builtin::AppBundle, &providers[0]],
    )
    .unwrap();
    assert!(matches!(
        crate::tree::select(&project_root, "pid:42")
            .unwrap()
            .attribution
            .unwrap()
            .source,
        crate::model::AttributionSource::Manifest
    ));
    let selected = crate::tree::select(&root, "pid:42").unwrap();
    let attribution = selected.attribution.as_ref().unwrap();
    assert_eq!(attribution.extension.as_ref().unwrap().provider, "example");
    let snapshot = Snapshot {
        schema: crate::model::JSON_SCHEMA_VERSION,
        extensions: vec![],
        captured_unix_ms: 0,
        sample_ms: 450,
        collection_ms: 450,
        system: SystemSummary::default(),
        scope: "pid:42".into(),
        root: selected,
        warnings: vec![],
    };
    let json = serde_json::to_value(&snapshot).unwrap();
    let schema: Value = serde_json::from_str(crate::output::JSON_SCHEMA).unwrap();
    let errors: Vec<_> = jsonschema::validator_for(&schema)
        .unwrap()
        .iter_errors(&json)
        .map(|error| error.to_string())
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
    for text in [
        serde_json::to_string(&json).unwrap(),
        crate::output::markdown(&snapshot),
    ] {
        assert!(text.contains("[filtered]"));
        assert!(!text.contains("fixture-secret-value"));
    }
    assert!(!dir.path().join("config.json").exists());
    dir.close().unwrap();
}

#[test]
fn protocol_failures_are_isolated_and_not_retried_within_the_round() {
    let dir = tempfile::tempdir().unwrap();
    let counter = dir.path().join("calls");
    let body = format!(
        "FILE *f=fopen({:?}, \"a\"); fputs(\"called\\n\", f); fclose(f); fputs(\"SECRET_STDERR_SENTINEL\", stderr); return 1",
        counter.to_str().unwrap()
    );
    let bad = fixture(dir.path(), "bad", &body);
    let healthy = fixture(
        dir.path(),
        "healthy",
        "char input[8192]; fread(input,1,sizeof(input),stdin); puts(\"{\\\"protocol_version\\\":1,\\\"provider\\\":\\\"healthy\\\",\\\"version\\\":\\\"1.0.0\\\",\\\"contribution\\\":{\\\"capability\\\":\\\"classify\\\",\\\"proposals\\\":[]}}\")",
    );
    let settings = super::super::default_settings().unwrap();
    let cancel = Cancellation::default();
    let context = Context::new(&settings, &cancel);
    let process = process(dir.path());
    let (_, _, diagnostics) = classify(&[&bad, &healthy], &context, std::slice::from_ref(&process));
    assert_eq!(diagnostics.len(), 1);
    assert!(
        dispatch(
            &bad,
            &context,
            Request::Details {
                process: &process,
                pointer_width: 8
            }
        )
        .is_err()
    );
    assert!(dispatch(&bad, &context, Request::Collect).is_err());
    assert_eq!(fs::read_to_string(&counter).unwrap().lines().count(), 1);
    assert!(
        !serde_json::to_string(&diagnostics)
            .unwrap()
            .contains("SECRET")
    );
    let fresh = Context::new(&settings, &cancel);
    assert!(dispatch(&bad, &fresh, Request::Collect).is_err());
    assert_eq!(fs::read_to_string(&counter).unwrap().lines().count(), 2);
    dir.close().unwrap();
}

#[test]
fn timeout_cancellation_and_pipe_limits_reap_owned_process_groups() {
    let dir = tempfile::tempdir().unwrap();
    let pid_file = dir.path().join("pid");
    let sleeping = fixture(
        dir.path(),
        "sleeping",
        &format!(
            "FILE *f=fopen({:?}, \"w\"); fprintf(f, \"%d\", getpid()); fclose(f); sleep(30)",
            pid_file.to_str().unwrap()
        ),
    );
    let settings = super::super::default_settings().unwrap();
    let cancel = Cancellation::default();
    let mut context = Context::new(&settings, &cancel);
    context.deadline = Instant::now() + Duration::from_secs(2);
    assert!(dispatch(&sleeping, &context, Request::Collect).is_err());
    let pid: i32 = fs::read_to_string(&pid_file).unwrap().parse().unwrap();
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    let flooding = fixture(
        dir.path(),
        "flood",
        "for(int i=0;i<9000000;i++) putchar('x'); fflush(stdout)",
    );
    let context = Context::new(&settings, &cancel);
    assert!(dispatch(&flooding, &context, Request::Collect).is_err());
    fs::remove_file(&pid_file).unwrap();
    let context = Context::new(&settings, &cancel);
    std::thread::scope(|scope| {
        let child = scope.spawn(|| dispatch(&sleeping, &context, Request::Collect));
        let start = Instant::now();
        while !pid_file.exists() && start.elapsed() < Duration::from_secs(2) {
            std::thread::sleep(Duration::from_millis(5));
        }
        cancel.cancel();
        assert!(child.join().unwrap().is_err());
    });
    let pid: i32 = fs::read_to_string(&pid_file).unwrap().parse().unwrap();
    assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    dir.close().unwrap();
}

#[test]
fn wire_validation_settings_scope_and_incompatible_versions_do_not_execute() {
    let dir = tempfile::tempdir().unwrap();
    let mut registry = super::super::settings_registry().unwrap();
    let secret: crate::settings::Definition = serde_json::from_value(json!({"key":"extensions.other.token","provider":"other","version":"1.0.0","type":"string","unit":null,"default":"SENTINEL","description":"private","sensitive":true})).unwrap();
    registry
        .register_extension("other", vec![secret], vec![])
        .unwrap();
    let settings = registry
        .resolve(&Config::default(), &BTreeMap::new())
        .unwrap();
    let cancel = Cancellation::default();
    let inspected = fixture(
        dir.path(),
        "inspect",
        "char input[8192]={0}; fread(input,1,sizeof(input)-1,stdin); assert(strstr(input,\"\\\"settings\\\":{}\")); assert(strstr(input,\"0.1.0\")); assert(!strstr(input,\"command\")); assert(!strstr(input,\"environment\")); puts(\"{\\\"protocol_version\\\":1,\\\"provider\\\":\\\"inspect\\\",\\\"version\\\":\\\"1.0.0\\\",\\\"contribution\\\":{\\\"capability\\\":\\\"classify\\\",\\\"proposals\\\":[]}}\")",
    );
    assert!(
        dispatch(
            &inspected,
            &Context::new(&settings, &cancel),
            Request::Classify(&[process(dir.path())])
        )
        .is_ok()
    );
    for body in [
        "puts(\"not json\")",
        "puts(\"{\\\"protocol_version\\\":2,\\\"provider\\\":\\\"bad\\\",\\\"version\\\":\\\"1.0.0\\\",\\\"contribution\\\":{\\\"capability\\\":\\\"classify\\\",\\\"proposals\\\":[]}}\")",
        "kill(getpid(), SIGKILL)",
        "puts(\"{\\\"protocol_version\\\":1,\\\"provider\\\":\\\"bad\\\",\\\"version\\\":\\\"9.0.0\\\",\\\"contribution\\\":{\\\"capability\\\":\\\"classify\\\",\\\"proposals\\\":[]}}\")",
        "puts(\"{\\\"protocol_version\\\":1,\\\"provider\\\":\\\"bad\\\",\\\"version\\\":\\\"1.0.0\\\",\\\"contribution\\\":{\\\"capability\\\":\\\"parsed_details\\\",\\\"identity\\\":{\\\"pid\\\":999,\\\"started_unix_us\\\":123},\\\"fields\\\":[]}}\")",
    ] {
        let bad = fixture(dir.path(), "bad", body);
        assert!(
            dispatch(
                &bad,
                &Context::new(&settings, &cancel),
                Request::Details {
                    process: &process(dir.path()),
                    pointer_width: 8
                }
            )
            .is_err()
        );
    }
    let marker = dir.path().join("executed");
    let mut incompatible = fixture(
        dir.path(),
        "incompatible",
        &format!(
            "FILE *f=fopen({:?}, \"w\"); fclose(f)",
            marker.to_str().unwrap()
        ),
    );
    incompatible
        .manifest
        .engines
        .insert("reswho".into(), ">99".into());
    assert!(
        dispatch(
            &incompatible,
            &Context::new(&settings, &cancel),
            Request::Collect
        )
        .is_err()
    );
    incompatible
        .manifest
        .engines
        .insert("reswho".into(), "*".into());
    incompatible.manifest.protocol_version = 99;
    assert!(
        dispatch(
            &incompatible,
            &Context::new(&settings, &cancel),
            Request::Collect
        )
        .is_err()
    );
    assert!(!marker.exists());
    dir.close().unwrap();
}

#[test]
fn external_collect_adapters_and_failure_snapshots_match_current_schema() {
    let dir = tempfile::tempdir().unwrap();
    let body = "char input[8192]; fread(input,1,sizeof(input),stdin); puts(\"{\\\"protocol_version\\\":1,\\\"provider\\\":\\\"good\\\",\\\"version\\\":\\\"1.0.0\\\",\\\"contribution\\\":{\\\"capability\\\":\\\"collect\\\",\\\"runtime\\\":{\\\"name\\\":\\\"good\\\",\\\"context\\\":null,\\\"endpoint_ref\\\":\\\"fixture-local\\\",\\\"daemon_id\\\":null,\\\"status\\\":\\\"available\\\",\\\"inventory_complete\\\":true,\\\"collection_ms\\\":0,\\\"unavailable\\\":[]},\\\"containers\\\":[]}}\")";
    let good = fixture(dir.path(), "good", body);
    let bad = fixture(dir.path(), "bad", "return 1");
    let settings = super::super::default_settings().unwrap();
    let cancel = Cancellation::default();
    let context = Context::new(&settings, &cancel);
    let mut root = crate::tree::build(vec![process(dir.path())]).unwrap();
    root.children
        .push(collect(&[&bad, &good], &context).unwrap());
    let snapshot = Snapshot {
        schema: crate::model::JSON_SCHEMA_VERSION,
        extensions: vec![],
        captured_unix_ms: 0,
        sample_ms: 250,
        collection_ms: 250,
        system: SystemSummary::default(),
        scope: "group:all".into(),
        root,
        warnings: vec![],
    };
    let json = serde_json::to_value(snapshot).unwrap();
    let schema: Value = serde_json::from_str(crate::output::JSON_SCHEMA).unwrap();
    let errors: Vec<_> = jsonschema::validator_for(&schema)
        .unwrap()
        .iter_errors(&json)
        .map(|error| error.to_string())
        .collect();
    assert!(errors.is_empty(), "{errors:?}");
    dir.close().unwrap();
}

#[test]
fn fractional_collect_response_and_remaining_capabilities_survive_wire_decoding() {
    let dir = tempfile::tempdir().unwrap();
    let runtime = Runtime {
        name: "fractional".into(),
        context: None,
        endpoint_ref: Some("fixture".into()),
        daemon_id: None,
        status: "available".into(),
        inventory_complete: true,
        collection_ms: 1,
        unavailable: vec![],
    };
    let container = Container {
        runtime: "fractional".into(),
        endpoint_ref: "fixture".into(),
        id: "one".into(),
        name: "one".into(),
        state: Some("running".into()),
        cpu_percent: Some(12.5),
        ..Default::default()
    };
    let response = Response {
        provider: "fractional".into(),
        version: "1.0.0".into(),
        contribution: Contribution::Collect {
            runtime,
            containers: vec![container],
        },
    };
    let mut wire = serde_json::to_value(response).unwrap();
    wire["protocol_version"] = json!(1);
    let bytes = serde_json::to_vec(&wire).unwrap();
    let decoded: WireResponse = serde_json::from_slice(&bytes).unwrap();
    let super::super::protocol::Contribution::Collect { containers, .. } = decoded.contribution
    else {
        panic!("expected collect");
    };
    assert_eq!(containers.unwrap()[0].cpu_percent, Some(12.5));
    let classify = json!({"protocol_version":1,"provider":"fractional","version":"1.0.0","contribution":{"capability":"classify","proposals":[]}});
    let precise: Value = serde_json::from_str("1.123456789012345678901").unwrap();
    let details = json!({"protocol_version":1,"provider":"fractional","version":"1.0.0","contribution":{"capability":"parsed_details","identity":{"pid":42,"started_unix_us":123},"fields":[{"key":"extensions.fractional.value","label":"Value","value":precise,"sensitive":false}]}});
    let body = format!(
        "char input[8192]={{0}}; fread(input,1,sizeof(input)-1,stdin); if(strstr(input,\"\\\"capability\\\":\\\"collect\\\"\")) puts({}); else if(strstr(input,\"\\\"capability\\\":\\\"classify\\\"\")) puts({}); else puts({})",
        serde_json::to_string(&String::from_utf8(bytes).unwrap()).unwrap(),
        serde_json::to_string(&classify.to_string()).unwrap(),
        serde_json::to_string(&details.to_string()).unwrap()
    );
    let healthy = fixture(dir.path(), "fractional", &body);
    let bad = fixture(dir.path(), "bad", "return 1");
    let settings = super::super::default_settings().unwrap();
    let cancel = Cancellation::default();
    let context = Context::new(&settings, &cancel);
    let results = dispatch_many(&[&bad, &healthy], &context, Request::Collect);
    assert!(results[0].1.is_err());
    assert!(results[1].1.is_ok());
    assert!(dispatch(&healthy, &context, Request::Classify(&[])).is_ok());
    let mut process = process(dir.path());
    assert!(attach_details(&[&healthy], &context, &mut process).is_empty());
    assert_eq!(process.extension_details[0].fields[0].value, precise);
    assert!(dispatch(&bad, &context, Request::Classify(&[])).is_err());
    dir.close().unwrap();
}

#[test]
fn required_unknown_features_and_scopes_skip_execution_and_preserve_raw_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let calls = dir.path().join("calls");
    let body = format!(
        "FILE *f=fopen({},\"a\"); fputs(\"called\",f); fclose(f);",
        serde_json::to_string(calls.to_str().unwrap()).unwrap()
    );
    let external = fixture(dir.path(), "future", &body);
    let settings = default_settings().unwrap();
    let cancel = Cancellation::default();
    for field in ["feature", "scope", "capability"] {
        let mut future = External {
            manifest: external.manifest.clone(),
        };
        match field {
            "feature" => {
                future.manifest.required_features = vec!["future.private_constraint".into()]
            }
            "scope" => future.manifest.collect_scopes = vec!["future.device".into()],
            _ => future.manifest.capabilities.push("future.inspect".into()),
        }
        let encoded = serde_json::to_value(&future.manifest).unwrap();
        let decoded: Manifest = serde_json::from_value(encoded.clone()).unwrap();
        assert_eq!(serde_json::to_value(&decoded).unwrap(), encoded);
        assert!(decoded.incompatibility().is_some());
        let context = Context::new(&settings, &cancel);
        assert!(dispatch(&future, &context, Request::Collect).is_err());
        assert!(!calls.exists());
        let state_path = dir.path().join(format!("{field}.json"));
        State::mutate(&state_path, |state| state.install(decoded)).unwrap();
        let state = State::load(&state_path).unwrap();
        assert_eq!(
            serde_json::to_value(&state.installed[0].manifest).unwrap(),
            encoded
        );
        assert!(
            !state
                .list(&settings, true)
                .last()
                .unwrap()
                .descriptor
                .enabled
        );
    }
    dir.close().unwrap();
}

#[test]
fn resource_only_requests_reject_unapproved_scopes_and_required_response_features() {
    let dir = tempfile::tempdir().unwrap();
    let response_path = dir.path().join("response.json");
    let calls_path = dir.path().join("calls");
    let body = format!(
        "FILE *calls=fopen({},\"a\"); fputs(\"called\\n\",calls); fclose(calls); char input[8192]; memset(input,0,sizeof(input)); fread(input,1,sizeof(input)-1,stdin); if(strstr(input,\"\\\"capability\\\":\\\"collect\\\"\")) assert(strstr(input,\"\\\"collect_scopes\\\":[\\\"resources\\\"]\")); FILE *f=fopen({},\"r\"); int ch; while((ch=fgetc(f))!=EOF) putchar(ch); fclose(f)",
        serde_json::to_string(calls_path.to_str().unwrap()).unwrap(),
        serde_json::to_string(response_path.to_str().unwrap()).unwrap()
    );
    let mut external = fixture(dir.path(), "fixture", &body);
    external.manifest.collect_scopes = vec!["containers".into(), "resources".into()];
    let settings = default_settings().unwrap();
    let cancel = Cancellation::default();
    let valid: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/extensions/resources-additive.json"
    ))
    .unwrap();
    for case in [
        "valid",
        "containers",
        "empty_containers",
        "required",
        "capability",
    ] {
        let mut value = valid.clone();
        match case {
            "containers" => {
                value["contribution"]["runtime"] = json!({"name":"fixture","context":null,"endpoint_ref":"local","daemon_id":null,"status":"available","inventory_complete":true,"collection_ms":0,"unavailable":[]})
            }
            "empty_containers" => value["contribution"]["containers"] = json!([]),
            "required" => value["required_features"] = json!(["unknown.privacy"]),
            "capability" => value["contribution"] = json!({"capability":"classify","proposals":[]}),
            _ => {}
        }
        fs::write(&response_path, value.to_string()).unwrap();
        let context = Context::new(&settings, &cancel);
        let result = dispatch(&external, &context, Request::CollectResources);
        assert_eq!(result.is_ok(), case == "valid", "{case}");
    }
    let role: Value =
        serde_json::from_str(include_str!("../../../tests/fixtures/extensions/role.json")).unwrap();
    for case in ["valid_role", "stale_role", "duplicate_role"] {
        let mut value = role.clone();
        if case == "stale_role" {
            value["contribution"]["proposals"][0]["identity"]["started_unix_us"] = json!(124);
        }
        if case == "duplicate_role" {
            let duplicate = value["contribution"]["proposals"][0].clone();
            value["contribution"]["proposals"]
                .as_array_mut()
                .unwrap()
                .push(duplicate);
        }
        fs::write(&response_path, value.to_string()).unwrap();
        let context = Context::new(&settings, &cancel);
        let process = process(Path::new("/tmp/work"));
        let result = dispatch(&external, &context, Request::Classify(&[process]));
        assert_eq!(result.is_ok(), case == "valid_role", "{case}");
    }
    let calls_before = fs::read_to_string(&calls_path).unwrap();
    external.manifest.collect_scopes = vec!["containers".into()];
    let context = Context::new(&settings, &cancel);
    assert!(dispatch(&external, &context, Request::CollectResources).is_err());
    assert_eq!(
        fs::read_to_string(&calls_path).unwrap(),
        calls_before,
        "container-only provider executed"
    );
    dir.close().unwrap();
}
