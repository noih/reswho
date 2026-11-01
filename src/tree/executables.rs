use super::stable_ref;
use crate::model::{Grouping, Node};
use std::{collections::BTreeMap, path::PathBuf};

pub(super) fn group(mut remainder: Node) -> Vec<Node> {
    let mut executables: BTreeMap<PathBuf, Node> = BTreeMap::new();
    let children = std::mem::take(&mut remainder.children);
    remainder.totals = Default::default();
    for child in children {
        let path = child
            .process
            .as_ref()
            .and_then(|process| process.executable.as_ref())
            .filter(|path| path.is_absolute() && path.file_name().is_some());
        if let Some(path) = path {
            let group = executables.entry(path.clone()).or_insert_with(|| {
                Node::group(
                    stable_ref("executable", &path.to_string_lossy()),
                    path.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                    vec![format!("exe {}", path.display())],
                    Grouping::Bin,
                )
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
    use super::*;
    use crate::{model::Process, tree};

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
