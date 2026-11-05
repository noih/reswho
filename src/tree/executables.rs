use crate::model::{Attribution, AttributionSource, Grouping, Node};
use std::collections::BTreeMap;

pub(super) fn group(
    mut remainder: Node,
    registry: &crate::extensions::placement::Registry,
    settings: &crate::settings::Resolved,
) -> Vec<Node> {
    let mut executables: BTreeMap<String, Node> = BTreeMap::new();
    let children = std::mem::take(&mut remainder.children);
    remainder.totals = Default::default();
    for child in children {
        let effective = |dimension: &str| {
            child
                .process
                .as_ref()
                .and_then(|process| process.tag_report.as_ref())
                .and_then(|report| {
                    report
                        .effective
                        .iter()
                        .find(|entry| entry.tag.dimension == dimension)
                })
        };
        let selected = child
            .process
            .as_ref()
            .and_then(|p| p.tag_report.as_ref())
            .and_then(|report| registry.selected(report, settings));
        let alias = selected.filter(|entry| {
            entry
                .tag
                .assignment
                .as_ref()
                .is_some_and(|a| a.group_key.is_some())
                && registry.ownership[&entry.tag.dimension].grouping == Grouping::Bin
        });
        let tag = alias.or_else(|| effective("bin"));
        let placed = selected.filter(|entry| {
            registry.ownership[&entry.tag.dimension].category == "unclassified"
                && entry.tag.dimension != "bin"
        });
        let path = tag
            .and_then(|entry| entry.tag.path.as_ref())
            .filter(|path| path.is_absolute() && path.file_name().is_some());
        if let (Some(path), Some(entry)) = (path, tag) {
            let (reference, label, grouping) = match placed {
                Some(placed) => (
                    placed
                        .tag
                        .assignment
                        .as_ref()
                        .and_then(|a| a.group_key.as_ref())
                        .map(|key| crate::extensions::host::stable_ref("executable", key))
                        .unwrap_or_else(|| {
                            crate::extensions::tags::ownership_ref(
                                &placed.tag.dimension,
                                &placed.tag.id,
                                path,
                            )
                        }),
                    placed.tag.label.clone(),
                    registry.ownership[&placed.tag.dimension].grouping.clone(),
                ),
                None => (
                    crate::extensions::tags::ownership_ref("bin", &entry.tag.id, path),
                    entry.tag.label.clone(),
                    Grouping::Bin,
                ),
            };
            let group = executables.entry(reference.clone()).or_insert_with(|| {
                let mut group = Node::group(
                    reference,
                    label,
                    vec![format!("exe {}", path.display())],
                    grouping,
                );
                group.attribution = Some(Attribution::new(
                    AttributionSource::Executable,
                    Some(path.clone()),
                ));
                group
            });
            group.totals.add(&child.totals);
            group.children.push(child);
        } else {
            remainder.totals.add(&child.totals);
            remainder.children.push(child);
        }
    }
    let mut groups: Vec<_> = executables
        .into_values()
        .map(|mut group| {
            let members: Vec<_> = group
                .children
                .iter()
                .filter_map(|child| child.process.as_ref())
                .collect();
            let consistent = |value: &dyn Fn(&crate::model::Process) -> Option<String>| {
                let first = value(members.first()?)?;
                members
                    .iter()
                    .all(|p| value(p).as_ref() == Some(&first))
                    .then_some(first)
            };
            if matches!(group.grouping, Some(Grouping::Bin)) {
                if let Some(name) = consistent(&|p| {
                    p.name_source
                        .as_ref()
                        .filter(|source| source.reason == crate::model::NameReason::Name)
                        .map(|_| p.name.clone())
                }) {
                    group.name = name;
                }
                group.version = members
                    .first()
                    .and_then(|p| p.version.clone())
                    .filter(|first| {
                        members.iter().all(|p| {
                            p.version.as_ref().is_some_and(|v| {
                                v.subject == first.subject && v.value == first.value
                            })
                        })
                    });
            }
            if group.totals.processes == 1 {
                group.children.pop().expect("single process bin")
            } else {
                group
            }
        })
        .collect();
    if !remainder.children.is_empty() {
        groups.push(remainder);
    }
    groups
}

#[cfg(test)]
mod tests {
    fn stable_ref(_: &str, path: &str) -> String {
        crate::extensions::tags::ownership_ref(
            "bin",
            &crate::extensions::tags::path_id("bin", std::path::Path::new(path)),
            std::path::Path::new(path),
        )
    }
    use crate::{model::Process, tree};
    fn group_fixture(node: crate::model::Node) -> Vec<crate::model::Node> {
        let settings = crate::extensions::default_settings().unwrap();
        let cancel = crate::snapshot::Cancellation::default();
        let context = crate::extensions::Context::new(&settings, &cancel);
        let registry = crate::extensions::placement::Registry::new(
            &crate::extensions::round_classifiers(),
            &context,
        );
        super::group(node, &registry, &settings)
    }

    fn process(pid: u32, executable: &str) -> Process {
        Process {
            pid,
            ppid: Some(1),
            started_unix_us: Some(u64::from(pid)),
            uptime_seconds: Some(1000 - u64::from(pid)),
            name: "same-name".into(),
            executable: Some(executable.into()),
            resident_bytes: Some(100),
            cpu_percent: Some(0.0),
            ..Process::default()
        }
    }

    #[test]
    fn fallback_bins_keep_subtrees_but_do_not_take_project_or_app_processes() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("go.mod"), "module example.test/tool").unwrap();
        let mut helper = process(11, "/cache/helper");
        helper.ppid = Some(10);
        helper.resident_bytes = None;
        let mut project = process(50, "/runtime/node");
        project.cwd = Some(temp.path().into());
        let unreadable = Process {
            pid: 70,
            name: "process 70".into(),
            ..Process::default()
        };
        let tree = tree::build(vec![
            process(10, "/bin/zsh"),
            helper,
            process(20, "/bin/zsh"),
            process(30, "/runtime/node"),
            process(31, "/runtime/node"),
            process(40, "/other/node"),
            project,
            process(60, "/Applications/Example.app/Contents/MacOS/main"),
            unreadable,
        ])
        .unwrap();
        assert_eq!(tree.totals.processes, 9);
        assert_eq!(tree.totals.resident_bytes, 700);
        assert_eq!(tree.totals.memory_unknown, 2);
        assert_eq!(
            tree::select(&tree, "group:projects")
                .unwrap()
                .totals
                .processes,
            1
        );
        assert_eq!(
            tree::select(&tree, "group:applications")
                .unwrap()
                .totals
                .processes,
            1
        );
        let unknown = tree::select(&tree, "group:unclassified").unwrap();
        assert_eq!(unknown.children.len(), 4);
        assert_eq!(unknown.totals.processes, 7);
        let shell_ref = stable_ref("executable", "/bin/zsh");
        let shell = tree::select(&tree, &shell_ref).unwrap();
        assert_eq!(shell.name, "zsh");
        assert_eq!(shell.totals.processes, 3);
        assert_eq!(shell.totals.memory_unknown, 1);
        assert_eq!(shell.totals.oldest_uptime_seconds, Some(990));
        assert_eq!(
            tree::select(&shell, "pid:10").unwrap().children[0].reference,
            "pid:11"
        );
        assert_eq!(
            tree::select(&tree, &stable_ref("executable", "/runtime/node"))
                .unwrap()
                .totals
                .processes,
            2
        );
        assert!(tree::select(&tree, &stable_ref("executable", "/other/node")).is_err());
        assert!(
            unknown
                .children
                .iter()
                .any(|node| node.reference == "pid:40")
        );
        assert!(tree::select(&tree, "pid:70").is_ok());
        let mut collapsed = tree;
        tree::limit_depth(&mut collapsed, 2);
        let shell = tree::select(&collapsed, &shell_ref).unwrap();
        assert_eq!(shell.omitted_children, 2);
        assert_eq!(shell.totals.processes, 3);
        assert!(shell.children.is_empty());
        temp.close().unwrap();
    }

    #[test]
    fn bin_groups_take_member_names_only_from_name_tags_shared_by_all_members() {
        let settings = crate::extensions::default_settings().unwrap();
        let cancel = crate::snapshot::Cancellation::default();
        let build = |named: Vec<u32>| {
            let identities = tree::versions::fixtures::Identities(
                named
                    .into_iter()
                    .map(|pid| (pid, Some("renamed"), None))
                    .collect(),
            );
            tree::build_with_providers(
                vec![process(10, "/opt/tool/run"), process(11, "/opt/tool/run")],
                &crate::extensions::Context::new(&settings, &cancel),
                &[&identities],
            )
            .unwrap()
        };
        let reference = stable_ref("executable", "/opt/tool/run");
        let group = |root| tree::select(&root, &reference).unwrap().name;
        assert_eq!(group(build(vec![10, 11])), "renamed");
        assert_eq!(group(build(vec![10])), "run");
    }

    #[test]
    fn work_renames_never_replace_the_bin_label() {
        let tree = tree::build(vec![
            process(10, "/opt/tool/run"),
            process(11, "/opt/tool/run"),
        ])
        .unwrap();
        let reference = stable_ref("executable", "/opt/tool/run");
        let mut group = tree::select(&tree, &reference).unwrap();
        for child in &mut group.children {
            let process = child.process.as_mut().unwrap();
            process.name = "work-label".into();
            process.name_source = Some(crate::model::NameSource {
                reason: crate::model::NameReason::Work,
                source: "node".into(),
                basis: "argv_script".into(),
            });
        }
        let mut remainder = crate::model::Node::group(
            "group:test".into(),
            "Unknown".into(),
            vec![],
            crate::model::Grouping::Unknown,
        );
        remainder.children = std::mem::take(&mut group.children);
        let groups = group_fixture(remainder);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].name, "run");
    }

    #[test]
    fn system_paths_keep_system_category_and_group_by_bin() {
        let tree = tree::build(vec![
            process(10, "/usr/libexec/example"),
            process(11, "/usr/libexec/example"),
            process(12, "/usr/libexec/other"),
        ])
        .unwrap();
        let system = tree::select(&tree, "group:system").unwrap();
        assert_eq!(system.children.len(), 1);
        assert_eq!(system.children[0].name, "System services");
        assert_eq!(system.children[0].children.len(), 2);
        assert_eq!(system.totals.processes, 3);
        assert_eq!(
            tree::select(&system, &stable_ref("executable", "/usr/libexec/example"))
                .unwrap()
                .totals
                .processes,
            2
        );
    }
}
