#[cfg(test)]
pub(crate) mod fixtures {
    use crate::extensions::{
        Capability, ClassificationEvidence, ClassificationProposal, Context, Contribution,
        Descriptor, Extension, ProcessIdentity, Request,
        tags::{Operation, Proposal, Tag},
    };
    use crate::model::{Process, Snapshot, SystemSummary};

    type Identity = (
        u32,
        Option<&'static str>,
        Option<(&'static str, &'static str)>,
    );

    pub(crate) struct Identities(pub Vec<Identity>);

    impl Extension for Identities {
        fn descriptor(&self) -> Descriptor {
            Descriptor {
                id: "fixture".into(),
                capabilities: vec![Capability::Classify],
                enabled: true,
                availability: "unknown",
            }
        }

        fn execute(&self, _: &Context<'_>, request: Request<'_>) -> anyhow::Result<Contribution> {
            let Request::Classify(facts) = request else {
                anyhow::bail!("unsupported");
            };
            let mut proposals = Vec::new();
            for (pid, name, version) in &self.0 {
                let Some(identity) = facts
                    .processes
                    .iter()
                    .find(|p| p.pid == *pid)
                    .and_then(ProcessIdentity::observed)
                else {
                    continue;
                };
                let tags =
                    name.map(|label| ("name", label, None, None))
                        .into_iter()
                        .chain(version.map(|(subject, value)| {
                            ("version", value, Some(subject), Some(value))
                        }));
                for (dimension, label, subject, value) in tags {
                    let tag = Tag {
                        hint: None,
                        dimension: dimension.into(),
                        id: format!("{dimension}:{label}"),
                        label: label.into(),
                        path: None,
                        basis: "executable".into(),
                        subject: subject.map(Into::into),
                        value: value.map(Into::into),
                        projection: None,
                        supplementary_to: None,
                        evidence_chain: vec![],
                        host_resource: None,
                        assignment: None,
                    };
                    let proposal = Proposal {
                        id: format!("{dimension}-{pid}"),
                        identity,
                        operation: Operation::Add { tag },
                    };
                    proposals.push(ClassificationProposal {
                        identity,
                        evidence: ClassificationEvidence::Tags { proposal },
                    });
                }
            }
            Ok(Contribution::Classify { proposals })
        }
    }

    fn process(pid: u32, name: &str, executable: Option<&str>) -> Process {
        Process {
            pid,
            ppid: Some(1),
            uid: Some(501),
            started_unix_us: Some(u64::from(pid)),
            uptime_seconds: Some(60),
            name: name.into(),
            executable: executable.map(Into::into),
            cpu_percent: Some(0.0),
            resident_bytes: Some(1024),
            ..Process::default()
        }
    }

    pub(crate) fn snapshot(scope: &str) -> Snapshot {
        let settings = crate::extensions::default_settings().unwrap();
        let cancel = crate::snapshot::Cancellation::default();
        let temp = tempfile::tempdir().unwrap();
        let fnm = temp
            .path()
            .canonicalize()
            .unwrap()
            .join("fnm/node-versions");
        let install = crate::extensions::builtin::node::tests::install;
        let old = install(&fnm.join("v24.18.0/installation"), "24.18.0");
        let new = install(&fnm.join("v24.22.1/installation"), "24.22.1");
        let user = unsafe { libc::geteuid() };
        let node = |pid: u32, executable: &std::path::Path| Process {
            uid: Some(user),
            ..process(pid, "node", executable.to_str())
        };
        let identities = Identities(vec![
            (30, None, Some(("rust", "1.99.1"))),
            (50, None, Some(("run", "1.0.0"))),
        ]);
        let root = crate::tree::build_with_providers(
            vec![
                node(10, &old),
                node(11, &old),
                node(12, &new),
                node(13, &new),
                process(
                    20,
                    "2.1.316",
                    Some("/Users/u/.local/share/claude/versions/2.1.316"),
                ),
                process(
                    30,
                    "rust-analyzer-proc-macro-srv",
                    Some("/opt/homebrew/Cellar/rust/1.99.1/libexec/rust-analyzer-proc-macro-srv"),
                ),
                process(40, "9.9.9", None),
                process(50, "run", Some("/opt/tool/run")),
                process(51, "run", Some("/opt/tool/run")),
            ],
            &Context::new(&settings, &cancel),
            &[&identities],
        )
        .unwrap();
        temp.close().unwrap();
        let root = if scope == "group:all" {
            root
        } else {
            crate::tree::select(&root, scope).unwrap()
        };
        Snapshot {
            schema: crate::model::JSON_SCHEMA_VERSION,
            extensions: vec![],
            captured_unix_ms: 1000,
            sample_ms: 250,
            collection_ms: 300,
            system: SystemSummary::default(),
            scope: scope.into(),
            warnings: vec![],
            root,
        }
    }
}

#[cfg(test)]
mod presentation_tests {
    use crate::output;

    #[test]
    fn markdown_shows_tag_versions_on_consistent_bin_groups_and_named_rows() {
        let md = output::markdown(&super::fixtures::snapshot("group:all"));
        assert!(md.contains("- node 24.18.0 [bin] group:"), "{md}");
        assert!(md.contains("- node 24.22.1 [bin] group:"), "{md}");
        assert!(md.contains("- run [bin] group:"), "{md}");
        assert!(!md.contains("1.0.0 [bin]"), "{md}");
        assert!(md.contains("- claude 2.1.316 pid:20 · "), "{md}");
        assert!(
            md.contains("- rust-analyzer-proc-macro-srv pid:30 · "),
            "{md}"
        );
        assert!(!md.contains("1.99.1"), "{md}");
        assert!(md.contains("- 9.9.9 pid:40 · "), "{md}");
        let detail = output::markdown(&super::fixtures::snapshot("pid:20"));
        assert!(detail.contains("claude 2.1.316 pid:20 · "), "{detail}");
        assert!(
            detail.contains(
                "> version 2.1.316 · subject claude · from versioned\\_install (executable)"
            ),
            "{detail}"
        );
        assert!(
            detail.contains(
                "reported name 2.1.316 · display name from versioned\\_install (executable)"
            ),
            "{detail}"
        );
        assert!(
            detail.contains("> Tags #executable · name claude · version claude 2.1.316"),
            "{detail}"
        );
        assert!(
            detail.contains("> add version 2.1.316 · subject claude · version:"),
            "{detail}"
        );
        let ordinary = output::markdown(&super::fixtures::snapshot("pid:30"));
        assert!(
            ordinary.contains("> add version 1.99.1 · subject rust · version:"),
            "{ordinary}"
        );
        assert!(
            ordinary.contains("> rejected: version subject mismatch · source fixture"),
            "{ordinary}"
        );
        assert!(ordinary.contains("> Tags #executable  "), "{ordinary}");
        assert!(ordinary.contains("rust-analyzer-proc-macro-srv pid:30 · "));
        assert!(ordinary.contains("> version ?"), "{ordinary}");
        assert!(!ordinary.contains("reported name"));
        let unreadable = output::markdown(&super::fixtures::snapshot("pid:40"));
        assert!(unreadable.contains("9.9.9 pid:40 · "));
        assert!(unreadable.contains("> version ?"));
    }

    #[test]
    fn process_rows_append_versions_only_for_matching_name_tags() {
        use crate::model::{NameReason, NameSource, Node, Process, Version};
        let row = |name_source: Option<NameSource>, subject: &str| {
            let mut node = Node::group(
                "pid:1".into(),
                "Claude".into(),
                vec![],
                crate::model::Grouping::Bin,
            );
            node.process = Some(Box::new(Process {
                name: "Claude".into(),
                name_source,
                version: Some(Version {
                    subject: subject.into(),
                    value: "2.1.316".into(),
                    basis: "executable".into(),
                    source: "versioned_install".into(),
                }),
                ..Process::default()
            }));
            node.display_name()
        };
        let named = || {
            Some(NameSource {
                reason: NameReason::Name,
                source: "versioned_install".into(),
                basis: "executable".into(),
            })
        };
        assert_eq!(row(named(), "claude"), "Claude 2.1.316");
        let work = Some(NameSource {
            reason: NameReason::Work,
            source: "node".into(),
            basis: "argv_script".into(),
        });
        assert_eq!(row(work, "claude"), "Claude");
        assert_eq!(row(named(), "node"), "Claude");
        assert_eq!(row(None, "claude"), "Claude");
    }

    #[test]
    fn json_always_carries_version_name_source_and_original_name() {
        let json = serde_json::to_value(super::fixtures::snapshot("group:all")).unwrap();
        let nodes: Vec<&serde_json::Value> = {
            let mut pending = vec![&json["selections"][0]["root"]];
            let mut all = Vec::new();
            while let Some(node) = pending.pop() {
                pending.extend(node["children"].as_array().unwrap());
                all.push(node);
            }
            all
        };
        let find = |predicate: &dyn Fn(&serde_json::Value) -> bool| {
            *nodes.iter().find(|node| predicate(node)).unwrap()
        };
        let bin = find(&|node| node["grouping"] == "bin" && node["version"]["value"] == "24.18.0");
        assert_eq!(bin["name"], "node");
        assert_eq!(
            bin["version"],
            serde_json::json!({"subject": "node", "value": "24.18.0", "basis": "executable", "source": "node"})
        );
        assert!(nodes.iter().any(|node| node["grouping"] == "bin"
            && node["name"] == "run"
            && node["version"].is_null()));
        let claude = find(&|node| node["reference"] == "pid:20");
        assert_eq!(claude["process"]["name"], "claude");
        assert_eq!(claude["process"]["original_name"], "2.1.316");
        assert_eq!(
            claude["process"]["name_source"],
            serde_json::json!({"reason": "name", "source": "versioned_install", "basis": "executable"})
        );
        assert_eq!(claude["process"]["version"]["subject"], "claude");
        assert_eq!(claude["process"]["version"]["value"], "2.1.316");
        let ordinary = find(&|node| node["reference"] == "pid:30");
        assert_eq!(
            ordinary["process"]["original_name"],
            serde_json::Value::Null
        );
        assert_eq!(ordinary["process"]["name_source"], serde_json::Value::Null);
        assert_eq!(ordinary["process"]["version"], serde_json::Value::Null);
        let unreadable = find(&|node| node["reference"] == "pid:40");
        assert_eq!(unreadable["process"]["name"], "9.9.9");
        assert_eq!(unreadable["process"]["version"], serde_json::Value::Null);
        assert!(nodes.iter().all(|node| node.get("version").is_some()));
        let schema: serde_json::Value = serde_json::from_str(output::JSON_SCHEMA).unwrap();
        let validator = jsonschema::validator_for(&schema).unwrap();
        for scope in ["group:all", "pid:20", "pid:40"] {
            let value = serde_json::to_value(super::fixtures::snapshot(scope)).unwrap();
            let errors: Vec<_> = validator
                .iter_errors(&value)
                .map(|e| e.to_string())
                .collect();
            assert!(errors.is_empty(), "{scope}: {errors:?}");
        }
    }
}
