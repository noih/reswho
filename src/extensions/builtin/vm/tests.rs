use super::*;

pub(crate) fn processes() -> Vec<Process> {
    [(10, "/Applications/Docker.app/Contents/MacOS/com.docker.backend"), (20, "/usr/local/libexec/container/plugins/container-runtime-linux/bin/container-runtime-linux"), (30, "/Applications/Editor.app/Contents/MacOS/editor"), (40, "/System/Library/Frameworks/Virtualization.framework/Versions/A/XPCServices/com.apple.Virtualization.VirtualMachine.xpc/Contents/MacOS/com.apple.Virtualization.VirtualMachine")]
        .into_iter().map(|(pid, exe)| Process { pid, ppid: Some(1), uid: Some(501), started_unix_us: Some(pid.into()), name: Path::new(exe).file_name().unwrap().to_string_lossy().into(), executable: Some(exe.into()), resident_bytes: Some(100), cpu_percent: Some(1.0), ..Default::default() }).collect()
}

#[test]
fn vm_evidence_degrades_without_guessing_conflicting_runtimes() {
    let p = processes();
    let facts = Facts::new(&p);
    let docker = "/Users/fixture/Library/Containers/com.docker.docker/Data/vms/0/data/Docker.raw";
    let apple = "/Users/fixture/Library/Application Support/com.apple.container/containers/owned/rootfs.ext4";
    for (paths, responsible, confidence, scope) in [
        (vec![docker], Some(10), "inferred", Some("docker")),
        (vec![docker], None, "partial", Some("docker")),
        (vec![], Some(10), "partial", Some("docker")),
        (vec![apple], Some(20), "inferred", Some("apple:owned")),
        (vec![apple], None, "partial", Some("apple:owned")),
        (vec![], Some(20), "partial", None),
        (vec![], Some(30), "unattributed", None),
        (vec![], None, "possible", None),
        (vec![docker], Some(20), "conflict", None),
        (vec![docker, apple], None, "conflict", None),
        (vec![docker, docker], Some(10), "inferred", Some("docker")),
        (
            vec![
                apple,
                "/Users/fixture/Library/Application Support/com.apple.container/containers/other/rootfs.ext4",
            ],
            Some(20),
            "conflict",
            None,
        ),
        (vec![docker], Some(30), "partial", Some("docker")),
    ] {
        let proposal = proposal(
            &facts,
            &p[3],
            Probe {
                paths: paths.into_iter().map(Into::into).collect(),
                ..Default::default()
            },
            responsible.map(|id| facts.by_pid[&id]),
            Path::new("/Users/fixture"),
        )
        .unwrap();
        let ClassificationEvidence::Tags { proposal } = proposal.evidence else {
            panic!()
        };
        let Operation::Add { tag } = proposal.operation else {
            panic!()
        };
        let resource = tag.host_resource.unwrap();
        assert_eq!(resource.confidence, confidence);
        assert_eq!(resource.scope.as_deref(), scope);
    }
}

#[test]
fn responsible_requires_current_identity_same_user_and_older_start() {
    let mut p = processes();
    for pid in [None, Some(0), Some(u32::MAX), Some(1), Some(40)] {
        assert!(vm::responsible(&p[3], pid, &Facts::new(&p).by_pid, |_| true).is_none());
    }
    assert!(vm::responsible(&p[3], Some(10), &Facts::new(&p).by_pid, |_| false).is_none());
    p[0].uid = Some(502);
    assert!(vm::responsible(&p[3], Some(10), &Facts::new(&p).by_pid, |_| true).is_none());
    p[0].uid = Some(501);
    p[0].started_unix_us = Some(99);
    assert!(vm::responsible(&p[3], Some(10), &Facts::new(&p).by_pid, |_| true).is_none());
}

#[test]
fn vm_adapter_paths_respect_boundaries_and_fd_limit() {
    let home = Path::new("/Users/fixture");
    for (path, expected) in [
        (
            "/Users/fixture/Library/Containers/com.docker.docker/Data/vms/0/data/Docker.raw",
            Some("docker"),
        ),
        (
            "/Users/fixture/Library/Application Support/com.apple.container/containers/owned/rootfs.ext4",
            Some("apple:owned"),
        ),
        (
            "/Users/other/Library/Containers/com.docker.docker/Data/vms/0/data/Docker.raw",
            None,
        ),
        (
            "/Users/fixture/Library/Containers/com.docker.docker.fake/Data/vms/0/data/Docker.raw",
            None,
        ),
        (
            "/Users/fixture/Library/Containers/com.docker.docker/Data/vms/../data/Docker.raw",
            None,
        ),
        (
            "/Users/fixture/Library/Containers/com.docker.docker/Data/vms/name/data/Docker.raw",
            None,
        ),
        (
            "/Users/fixture/Library/Application Support/com.apple.container/containers/owned/../other/rootfs.ext4",
            None,
        ),
        (
            "/Users/fixture/Library/Application Support/com.apple.container/containers/owned/rootfs.ext4.extra",
            None,
        ),
    ] {
        assert_eq!(disk(Path::new(path), home).as_deref(), expected, "{path}");
    }
    for path in [
        "/Applications/Docker.app.fake/Contents/MacOS/com.docker.backend",
        "/Applications/Other.app/Contents/MacOS/com.docker.backend",
        "/Applications/Docker.app/Contents/MacOS/com.docker.backend.extra",
        "/usr/local/libexec/container/plugins/other/bin/container-runtime-linux",
        "/usr/local/libexec/container/plugins/container-runtime-linux/bin/../container-runtime-linux",
    ] {
        assert!(helper(Path::new(path)).is_none(), "{path}");
    }
    let p = processes();
    let facts = Facts::new(&p);
    let mut paths = vec![PathBuf::from("/unrecognized"); 64];
    paths.push(home.join("Library/Containers/com.docker.docker/Data/vms/0/data/Docker.raw"));
    let proposal = proposal(
        &facts,
        &p[3],
        Probe {
            paths,
            ..Default::default()
        },
        None,
        home,
    )
    .unwrap();
    let ClassificationEvidence::Tags { proposal } = proposal.evidence else {
        panic!()
    };
    let Operation::Add { tag } = proposal.operation else {
        panic!()
    };
    assert_eq!(tag.host_resource.unwrap().confidence, "possible");
}

pub(crate) struct Fixture {
    pub(crate) apple: bool,
}
impl crate::extensions::Extension for Fixture {
    fn descriptor(&self) -> crate::extensions::Descriptor {
        crate::extensions::Descriptor {
            id: "vm_fixture".into(),
            capabilities: vec![Capability::Classify],
            enabled: true,
            availability: "available",
        }
    }
    fn registration(&self) -> Registration {
        let mut r = registration();
        for o in &mut r.ownership {
            o.dimension = o
                .dimension
                .replace("extensions.vm.", "extensions.vm_fixture.");
        }
        r
    }
    fn execute(&self, _: &Context<'_>, request: Request<'_>) -> anyhow::Result<Contribution> {
        let Request::Classify(facts) = request else {
            anyhow::bail!("unsupported")
        };
        let process = facts.by_pid[&40];
        let disk = if self.apple {
            "/Users/fixture/Library/Application Support/com.apple.container/containers/owned/rootfs.ext4"
        } else {
            "/Users/fixture/Library/Containers/com.docker.docker/Data/vms/0/data/Docker.raw"
        };
        let responsible = facts.by_pid[&if self.apple { 20 } else { 10 }];
        let mut proposal = proposal(
            facts,
            process,
            Probe {
                paths: vec![disk.into()],
                ..Default::default()
            },
            Some(responsible),
            Path::new("/Users/fixture"),
        )
        .unwrap();
        if let ClassificationEvidence::Tags { proposal } = &mut proposal.evidence
            && let Operation::Add { tag } = &mut proposal.operation
        {
            tag.dimension = tag
                .dimension
                .replace("extensions.vm.", "extensions.vm_fixture.");
            tag.id = tag.id.replace("extensions.vm.", "extensions.vm_fixture.");
        }
        Ok(Contribution::Classify {
            proposals: vec![proposal],
        })
    }
}

pub(crate) fn fixture_snapshot(apple: bool, enabled: bool) -> crate::model::Snapshot {
    use crate::{
        containers::{Container, Runtime},
        model::ResourceConfiguration,
    };
    let mut settings = crate::extensions::default_settings().unwrap();
    settings.set_builtin_enabled(ID, false).unwrap();
    let cancel = crate::snapshot::Cancellation::default();
    let context = Context::new(&settings, &cancel);
    let provider = Fixture { apple };
    let providers: Vec<&dyn crate::extensions::Extension> =
        if enabled { vec![&provider] } else { vec![] };
    let mut root = crate::tree::build_with_providers(processes(), &context, &providers).unwrap();
    assert!(root.diagnostics.is_empty(), "{:?}", root.diagnostics);
    let name = if apple { "apple" } else { "docker" };
    let configuration = ResourceConfiguration {
        scope: if apple { "container_limit" } else { "vm" }.into(),
        cpus: Some(2),
        memory_bytes: Some(268435456),
        cpu_overhead: apple.then_some(1),
    };
    let mut runtime = Runtime {
        disk_usage: None,
        metrics_scope: Some("in_vm".into()),
        host_scope: (!apple).then(|| "docker".into()),
        configuration: (!apple).then(|| Box::new(configuration.clone())),
        name: name.into(),
        context: Some("fixture".into()),
        endpoint_ref: Some("group:fixture".into()),
        daemon_id: None,
        status: "available".into(),
        inventory_complete: true,
        collection_ms: 0,
        unavailable: vec![],
    };
    let mut containers: Vec<Container> = ["owned", "second"]
        .map(|id| Container {
            runtime: name.into(),
            host_scope: apple.then(|| format!("apple:{id}")),
            configuration: apple.then(|| Box::new(configuration.clone())),
            metrics_scope: Some("in_vm".into()),
            endpoint_ref: "group:fixture".into(),
            id: id.into(),
            name: id.into(),
            state: Some("running".into()),
            cpu_percent: Some(0.5),
            memory_bytes: Some(99),
            ..Default::default()
        })
        .into();
    crate::containers::retain_host_metadata(enabled, &mut runtime, &mut containers);
    crate::tree::compose_category(
        &mut root,
        crate::containers::build_all(vec![(runtime, containers)]).unwrap(),
    )
    .unwrap();
    crate::tree::link_host_resources(&mut root);
    crate::model::Snapshot {
        schema: crate::model::JSON_SCHEMA_VERSION,
        captured_unix_ms: 1,
        sample_ms: 1,
        collection_ms: 1,
        system: Default::default(),
        extensions: vec![],
        warnings: vec![],
        scope: "group:all".into(),
        root,
    }
}

#[test]
fn vm_links_do_not_aggregate_host_usage_or_leave_stale_refs_when_disabled() {
    for apple in [false, true] {
        let enabled = fixture_snapshot(apple, true);
        let disabled = fixture_snapshot(apple, false);
        assert_eq!(enabled.root.totals.processes, 4);
        assert_eq!(enabled.root.totals.resident_bytes, 400);
        assert_eq!(
            serde_json::to_value(&enabled.root.totals).unwrap(),
            serde_json::to_value(&disabled.root.totals).unwrap()
        );
        crate::tree::validate_references(&enabled.root).unwrap();
        let mut pending = vec![&enabled.root];
        let mut linked = 0;
        let mut vms = 0;
        while let Some(node) = pending.pop() {
            vms += usize::from(node.process.as_ref().is_some_and(|p| p.pid == 40));
            if !node.host_resources.is_empty() {
                linked += node.host_resources.len();
                assert_eq!(node.host_resources[0].reference, "pid:40");
                let target =
                    crate::tree::select(&enabled.root, &node.host_resources[0].reference).unwrap();
                assert_eq!(
                    target.process.unwrap().started_unix_us,
                    Some(node.host_resources[0].identity.started_unix_us)
                );
                assert_eq!(node.totals.processes, 0);
                assert_eq!(node.container.is_some(), apple);
                assert_eq!(node.runtime.is_some(), !apple);
            }
            pending.extend(&node.children);
        }
        assert_eq!(linked, 1);
        assert_eq!(vms, 1);
        assert_eq!(
            crate::tree::select(&enabled.root, "group:containers")
                .unwrap()
                .container_totals
                .as_ref()
                .unwrap()
                .known_memory_bytes,
            198
        );
        let path = Path::new(if apple {
            "/usr/local/libexec/container/plugins/container-runtime-linux/bin/container-runtime-linux"
        } else {
            "/Applications/Docker.app"
        });
        let dimension = if apple { "bin" } else { "app" };
        let group = crate::tree::select(
            &enabled.root,
            &crate::extensions::tags::ownership_ref(dimension, &path_id(dimension, path), path),
        )
        .unwrap_or_else(|e| panic!("apple={apple}: {e}\n{}", crate::output::markdown(&enabled)));
        assert_eq!(group.totals.processes, 2);
        let vm = crate::tree::select(&enabled.root, "pid:40").unwrap();
        assert!(vm.attribution.as_ref().unwrap().inferred);
        assert_eq!(
            vm.attribution.as_ref().unwrap().via_pid,
            Some(if apple { 20 } else { 10 })
        );
        assert_eq!(
            vm.process
                .unwrap()
                .host_resource
                .unwrap()
                .configuration
                .unwrap()
                .scope,
            if apple { "container_limit" } else { "vm" }
        );
        let disabled_json = serde_json::to_string(&disabled).unwrap();
        assert!(!disabled_json.contains("host_resource"));
        assert!(!disabled_json.contains("configuration"));
        assert!(!disabled_json.contains("metrics_scope"));
        let mut pending = vec![&disabled.root];
        while let Some(node) = pending.pop() {
            assert!(node.host_resources.is_empty());
            assert!(
                node.process
                    .as_ref()
                    .is_none_or(|p| p.host_resource.is_none())
            );
            pending.extend(&node.children);
        }
        let value = serde_json::to_value(&enabled).unwrap();
        let schema: serde_json::Value = serde_json::from_str(crate::output::JSON_SCHEMA).unwrap();
        let validator = jsonschema::validator_for(&schema).unwrap();
        let errors: Vec<_> = validator
            .iter_errors(&value)
            .map(|e| e.to_string())
            .collect();
        assert!(errors.is_empty(), "{}", errors.join("\n"));
        let mut runtime_scope = enabled.clone();
        runtime_scope.root = crate::tree::select(&enabled.root, "group:containers")
            .unwrap()
            .children
            .remove(0);
        runtime_scope.scope = runtime_scope.root.reference.clone();
        let runtime_md = crate::output::markdown_with_depth(&runtime_scope, 0);
        assert!(runtime_md.contains("in-VM usage: CPU"), "{runtime_md}");
        let md = crate::output::markdown(&enabled);
        assert!(md.contains("VM host pid:40"), "{md}");
        assert!(md.contains("in-VM"), "{md}");
        if apple {
            assert!(md.contains("VM total RAM unknown"), "{md}");
            assert!(md.contains("CPUs 2 + VM overhead 1 = VM 3"), "{md}");
        } else {
            assert!(md.contains("Docker [app]"), "{md}");
        }
    }
}

#[cfg(target_os = "macos")]
#[test]
fn vm_native_fd_probe_is_bounded_and_owned_files_are_removed() {
    unsafe extern "C" {
        fn reswho_vm_probe(pid: i32, paths: *mut libc::c_char, limited: *mut i32) -> i32;
    }
    let root = tempfile::tempdir().unwrap();
    let files: Vec<_> = (0..65)
        .map(|i| std::fs::File::create(root.path().join(format!("fd-{i}"))).unwrap())
        .collect();
    let mut paths = [[0i8; 1024]; 64];
    let mut limited = 0;
    let count = unsafe {
        reswho_vm_probe(
            std::process::id() as i32,
            paths.as_mut_ptr().cast(),
            &mut limited,
        )
    };
    assert!((1..=64).contains(&count), "count {count}");
    let found: Vec<_> = paths
        .iter()
        .take(count as usize)
        .map(|p| {
            unsafe { std::ffi::CStr::from_ptr(p.as_ptr()) }
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert!(!found.iter().any(|p| p.ends_with("/fd-64")));
    assert_eq!(limited, 1);
    drop(files);
    let path = root.path().to_path_buf();
    root.close().unwrap();
    assert!(!path.exists());
}

#[test]
fn vm_sensitive_metadata_is_not_retained_in_proposals_or_rendered_output() {
    let definition=serde_json::from_value(serde_json::json!({"key":"extensions.fixture.secret","provider":"fixture","type":"string","description":"fixture","default":"rootfs.ext4","sensitive":true})).unwrap();
    let mut registry = crate::extensions::settings_registry().unwrap();
    registry
        .register_extension("fixture", vec![definition], vec![])
        .unwrap();
    let mut settings = registry
        .resolve(&crate::settings::Config::default(), &Default::default())
        .unwrap();
    settings.set_builtin_enabled(ID, false).unwrap();
    let cancel = crate::snapshot::Cancellation::default();
    let context = Context::new(&settings, &cancel);
    let root =
        crate::tree::build_with_providers(processes(), &context, &[&Fixture { apple: true }])
            .unwrap();
    let value = serde_json::to_string(&root).unwrap();
    assert!(!value.contains("rootfs.ext4"));
    assert!(
        crate::tree::select(&root, "pid:40")
            .unwrap()
            .process
            .unwrap()
            .host_resource
            .is_none()
    );
    assert!(
        context
            .report
            .lock()
            .unwrap()
            .iter()
            .any(|r| r.provider == "vm_fixture" && r.result == "failed")
    );
}

#[test]
fn vm_app_alias_suppresses_a_same_app_launcher_without_changing_ownership() {
    let mut p = processes();
    p[3].ppid = Some(10);
    let mut settings = crate::extensions::default_settings().unwrap();
    settings.set_builtin_enabled(ID, false).unwrap();
    let cancel = crate::snapshot::Cancellation::default();
    let root = crate::tree::build_with_providers(
        p,
        &Context::new(&settings, &cancel),
        &[&Fixture { apple: false }],
    )
    .unwrap();
    let node = crate::tree::select(&root, "pid:40").unwrap();
    assert_eq!(
        node.attribution.unwrap().path.as_deref(),
        Some(Path::new("/Applications/Docker.app"))
    );
    let process = node.process.unwrap();
    assert!(process.host_resource.is_some());
    assert!(process.supplementary_labels.is_empty());
    assert!(
        process
            .tag_report
            .unwrap()
            .proposals
            .iter()
            .all(|p| p.source != "launched_by")
    );
}
