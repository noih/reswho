use crate::model::{Grouping, Node, Process, Totals};
use anyhow::{Result, bail};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::{Path, PathBuf},
};

mod applications;
mod executables;
#[cfg(test)]
mod project_tests;
pub(crate) mod projects;

#[derive(Clone, Debug)]
struct Assignment {
    category: &'static str,
    category_name: &'static str,
    key: String,
    name: String,
    evidence: String,
    project: Option<projects::Project>,
}

pub(crate) fn stable_ref(category: &str, key: &str) -> String {
    let digest = Sha256::digest(format!("{category}\0{key}"));
    format!("group:{}", &format!("{digest:x}")[..16])
}

fn app_bundle(path: &Path) -> Option<PathBuf> {
    let mut bundle = PathBuf::new();
    for component in path.components() {
        bundle.push(component);
        let name = bundle
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        if (name.ends_with(".app") || name.ends_with(".app.bundle"))
            && path
                .strip_prefix(&bundle)
                .ok()
                .is_some_and(|rest| rest.starts_with("Contents"))
        {
            return Some(bundle);
        }
    }
    None
}

fn assignment(process: &Process, cache: &mut projects::Discovery) -> Assignment {
    if let Some(bundle) = process.executable.as_deref().and_then(app_bundle) {
        let key = bundle.to_string_lossy().into_owned();
        let filename = bundle.file_name().unwrap_or_default().to_string_lossy();
        let name = filename
            .trim_end_matches(".bundle")
            .trim_end_matches(".app")
            .to_owned();
        return Assignment {
            category: "applications",
            category_name: "Applications",
            name,
            evidence: format!("Application bundle: {key}"),
            key,
            project: None,
        };
    }
    if let Some(root) = process.cwd.as_deref().and_then(|cwd| cache.find(cwd)) {
        let key = root.root.to_string_lossy().into_owned();
        return Assignment {
            category: "projects",
            category_name: "Projects",
            name: root
                .root
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            evidence: root.evidence(),
            key,
            project: Some(root),
        };
    }
    let executable = process.executable.as_deref();
    if executable.is_some_and(|path| {
        ["/System", "/usr/libexec", "/usr/sbin", "/sbin"]
            .iter()
            .any(|root| path.starts_with(root))
    }) {
        return Assignment {
            category: "system",
            category_name: "System",
            key: "services".into(),
            name: "System services".into(),
            evidence: "System executable location".into(),
            project: None,
        };
    }
    Assignment {
        category: "unclassified",
        category_name: "Unclassified",
        key: "unknown".into(),
        name: "Unknown ownership".into(),
        evidence: "No application or project identified from available paths".into(),
        project: None,
    }
}

fn ancestor_assignment<'a>(
    process: &Process,
    processes: &HashMap<u32, &Process>,
    assignments: &'a HashMap<u32, Assignment>,
) -> Option<(u32, &'a Assignment)> {
    let mut current = process;
    let mut visited = HashSet::from([process.pid]);
    for _ in 0..128 {
        let pid = current.ppid.filter(|pid| *pid > 1)?;
        if !visited.insert(pid) {
            return None;
        }
        let parent = *processes.get(&pid)?;
        if current.uid? != parent.uid?
            || parent.started_unix_us? > current.started_unix_us?
            || current.cwd.as_ref()? != parent.cwd.as_ref()?
        {
            return None;
        }
        let assignment = assignments.get(&pid)?;
        match assignment.category {
            "applications" | "projects" => return Some((pid, assignment)),
            "unclassified" => current = parent,
            _ => return None,
        }
    }
    None
}

pub fn build(processes: Vec<Process>) -> Result<Node> {
    let mut seen = HashSet::new();
    let mut cache = projects::Discovery::default();
    let mut buckets: BTreeMap<String, (Assignment, Vec<Process>)> = BTreeMap::new();
    for process in &processes {
        if !seen.insert(process.pid) {
            bail!("duplicate PID {} in snapshot", process.pid);
        }
    }
    let direct: HashMap<_, _> = processes
        .iter()
        .map(|process| (process.pid, assignment(process, &mut cache)))
        .collect();
    let by_pid: HashMap<_, _> = processes
        .iter()
        .map(|process| (process.pid, process))
        .collect();
    let mut assignments = HashMap::new();
    let mut evidence = HashMap::new();
    for process in &processes {
        let mut selected = direct[&process.pid].clone();
        let mut reasons = vec![selected.evidence.clone()];
        if selected.category == "unclassified" {
            if let Some((ancestor, inferred)) = ancestor_assignment(process, &by_pid, &direct) {
                selected = inferred.clone();
                reasons = vec![format!(
                    "Inferred from ancestor pid:{ancestor}: {}; same UID and working directory along observed parent chain",
                    inferred.evidence
                )];
            }
        } else if selected.category == "applications"
            && let Some(root) = process.cwd.as_deref().and_then(|cwd| cache.find(cwd))
        {
            reasons.push(format!(
                "Working-directory project candidate: {}; application grouping retained",
                root.root.display()
            ));
        }
        assignments.insert(process.pid, selected);
        evidence.insert(process.pid, reasons);
    }
    for process in processes {
        let assignment = assignments
            .remove(&process.pid)
            .expect("assignment for every process");
        let reference = assignment.project.as_ref().map_or_else(
            || stable_ref(assignment.category, &assignment.key),
            projects::Project::reference,
        );
        let bucket = buckets
            .entry(reference)
            .or_insert_with(|| (assignment.clone(), Vec::new()));
        if bucket.0.key != assignment.key || bucket.0.category != assignment.category {
            bail!("group identity collision");
        }
        bucket.1.push(process);
    }
    let mut categories = BTreeMap::new();
    let mut project_groups = Vec::new();
    let mut bundles = Vec::new();
    for (reference, (assignment, processes)) in buckets {
        let grouping = match assignment.category {
            "applications" => Grouping::App,
            "projects" => assignment
                .project
                .as_ref()
                .expect("project assignment")
                .grouping(),
            "system" => Grouping::System,
            _ => Grouping::Unknown,
        };
        let mut group = Node::group(
            reference,
            assignment.name,
            vec![assignment.evidence],
            grouping,
        );
        group.children = process_forest(processes, &mut evidence);
        for child in &group.children {
            group.totals.add(&child.totals);
        }
        let category = categories.entry(assignment.category).or_insert_with(|| {
            Node::group(
                format!("group:{}", assignment.category),
                assignment.category_name.into(),
                Vec::new(),
                Grouping::Category,
            )
        });
        category.totals.add(&group.totals);
        if let Some(project) = assignment.project {
            project_groups.push((project, group));
        } else if assignment.category == "applications" {
            bundles.push((PathBuf::from(assignment.key), group));
        } else if assignment.category == "unclassified" {
            category.children.extend(executables::group(group));
        } else if assignment.category == "system" {
            let mut services = Node::group(
                group.reference.clone(),
                group.name.clone(),
                group.evidence.clone(),
                Grouping::System,
            );
            services.totals = group.totals.clone();
            services.children = executables::group(group);
            category.children.push(services);
        } else {
            category.children.push(group);
        }
    }
    if let Some(category) = categories.get_mut("projects") {
        category.children = projects::group(project_groups);
    }
    if let Some(applications) = categories.get_mut("applications") {
        applications.children = applications::group(bundles);
    }
    let mut root = Node::group(
        "group:all".into(),
        "Resources".into(),
        Vec::new(),
        Grouping::Category,
    );
    root.children = categories.into_values().collect();
    for category in &root.children {
        root.totals.add(&category.totals);
    }
    validate_references(&root)?;
    sort_children(&mut root);
    Ok(root)
}

pub fn validate_references(root: &Node) -> Result<()> {
    let mut references = HashSet::new();
    let mut pending = vec![root];
    while let Some(node) = pending.pop() {
        if !references.insert(&node.reference) {
            bail!("resource identity collision: {}", node.reference);
        }
        pending.extend(&node.children);
    }
    Ok(())
}

fn sort_children(node: &mut Node) {
    node.children.sort_by(|a, b| {
        b.totals
            .resident_bytes
            .cmp(&a.totals.resident_bytes)
            .then(a.reference.cmp(&b.reference))
    });
    for child in &mut node.children {
        sort_children(child);
    }
}

fn process_forest(processes: Vec<Process>, evidence: &mut HashMap<u32, Vec<String>>) -> Vec<Node> {
    let mut by_pid: BTreeMap<u32, Process> = processes
        .into_iter()
        .map(|process| (process.pid, process))
        .collect();
    let mut children: BTreeMap<Option<u32>, Vec<u32>> = BTreeMap::new();
    for process in by_pid.values() {
        let parent = process.ppid.filter(|pid| {
            by_pid.get(pid).is_some_and(|parent| {
                parent
                    .started_unix_us
                    .zip(process.started_unix_us)
                    .is_some_and(|(parent_start, child_start)| parent_start <= child_start)
                    && *pid != process.pid
            })
        });
        children.entry(parent).or_default().push(process.pid);
    }
    let mut roots = Vec::new();
    for pid in children.get(&None).cloned().unwrap_or_default() {
        if let Some(node) = process_node(pid, &mut by_pid, &children, evidence, 0) {
            roots.push(node);
        }
    }
    while let Some(pid) = by_pid.keys().next().copied() {
        if let Some(mut node) = process_node(pid, &mut by_pid, &children, evidence, 0) {
            node.evidence
                .push("Parent chain cycle or depth limit; displayed as a separate root".into());
            roots.push(node);
        }
    }
    roots
}

fn process_node(
    pid: u32,
    processes: &mut BTreeMap<u32, Process>,
    children: &BTreeMap<Option<u32>, Vec<u32>>,
    evidence: &mut HashMap<u32, Vec<String>>,
    depth: usize,
) -> Option<Node> {
    let process = processes.remove(&pid)?;
    let mut totals = Totals::default();
    totals.add_process(&process);
    let mut node = Node {
        reference: format!("pid:{pid}"),
        name: process.name.clone(),
        kind: "process",
        grouping: None,
        evidence: evidence.remove(&pid).unwrap_or_default(),
        process: Some(process),
        container: None,
        runtime: None,
        container_totals: None,
        totals,
        children: Vec::new(),
        omitted_children: 0,
    };
    if depth < 128 {
        for child in children.get(&Some(pid)).into_iter().flatten() {
            if let Some(child) = process_node(*child, processes, children, evidence, depth + 1) {
                node.totals.add(&child.totals);
                node.children.push(child);
            }
        }
    }
    Some(node)
}

pub fn select(root: &Node, reference: &str) -> Result<Node> {
    let mut pending = vec![(root, Vec::new())];
    while let Some((node, context)) = pending.pop() {
        if node.reference == reference {
            let mut selected = node.clone();
            selected.evidence.extend(context);
            return Ok(selected);
        }
        let mut context = context;
        if node.kind == "group" && node.reference != "group:all" {
            context.push(format!("Grouped under: {} ({})", node.name, node.reference));
            context.extend(node.evidence.clone());
            if let Some(runtime) = &node.runtime {
                context.extend(
                    runtime
                        .unavailable
                        .iter()
                        .map(|issue| format!("Runtime unavailable: {issue}")),
                );
            }
        }
        pending.extend(node.children.iter().map(|child| (child, context.clone())));
    }
    bail!("target not found: {reference}")
}

pub fn limit_depth(node: &mut Node, depth: usize) {
    if depth == 0 {
        node.omitted_children = node.children.len();
        node.children.clear();
    } else {
        for child in &mut node.children {
            limit_depth(child, depth - 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn process(pid: u32, parent: u32, memory: Option<u64>) -> Process {
        Process {
            pid,
            ppid: Some(parent),
            started_unix_us: Some(u64::from(pid)),
            name: "node".into(),
            resident_bytes: memory,
            cpu_percent: Some(0.0),
            ..Process::default()
        }
    }

    #[test]
    fn descendants_count_once_and_depth_does_not_change_totals() {
        let mut tree = build(vec![
            process(10, 1, Some(100)),
            process(11, 10, None),
            process(12, 11, Some(300)),
        ])
        .unwrap();
        assert_eq!(tree.totals.processes, 3);
        assert_eq!(tree.totals.resident_bytes, 400);
        assert_eq!(tree.totals.memory_unknown, 1);
        let mut branch = select(&tree, "pid:10").unwrap();
        limit_depth(&mut branch, 1);
        assert_eq!(branch.children.len(), 1);
        assert_eq!(branch.children[0].omitted_children, 1);
        assert_eq!(branch.totals.resident_bytes, 400);
        limit_depth(&mut tree, 0);
        assert_eq!(tree.totals.processes, 3);
        assert!(tree.children.is_empty());
        assert_eq!(tree.omitted_children, 1);
    }

    #[test]
    fn git_directories_and_worktree_files_have_distinct_stable_groups() {
        let temp = tempfile::tempdir().unwrap();
        let main = temp.path().join("main");
        let worktree = temp.path().join("worktree");
        std::fs::create_dir_all(main.join(".git")).unwrap();
        std::fs::create_dir_all(main.join("src")).unwrap();
        std::fs::create_dir(&worktree).unwrap();
        std::fs::write(
            worktree.join(".git"),
            "gitdir: ../main/.git/worktrees/example",
        )
        .unwrap();
        let mut a = process(10, 1, Some(100));
        a.cwd = Some(main.join("src"));
        let mut b = process(11, 10, Some(200));
        b.cwd = Some(worktree);
        let first = build(vec![a.clone(), b.clone()]).unwrap();
        a.resident_bytes = Some(900);
        let second = build(vec![b, a]).unwrap();
        let refs = |root: &Node| -> HashSet<String> {
            root.children[0]
                .children
                .iter()
                .map(|node| node.reference.clone())
                .collect()
        };
        assert_eq!(refs(&first), refs(&second));
        assert_eq!(refs(&first).len(), 2);
        assert!(select(&first, "pid:10").unwrap().children.is_empty());
        assert_eq!(
            select(&first, "pid:11").unwrap().process.unwrap().ppid,
            Some(10)
        );
    }

    #[test]
    fn common_git_directory_groups_checkouts_without_merging_clones() {
        let temp = tempfile::tempdir().unwrap();
        let main = temp.path().join("owner/project");
        let a = temp.path().join("checkouts/a");
        let b = temp.path().join("checkouts/b");
        let clone = temp.path().join("other/project");
        let common = main.join(".git");
        for path in [
            &a,
            &b,
            &clone.join(".git"),
            &common.join("worktrees/a"),
            &common.join("worktrees/b"),
        ] {
            std::fs::create_dir_all(path).unwrap();
        }
        std::fs::write(
            a.join(".git"),
            format!("gitdir: {}\n", common.join("worktrees/a").display()),
        )
        .unwrap();
        std::fs::write(
            b.join(".git"),
            "gitdir: ../../owner/project/.git/worktrees/b\n",
        )
        .unwrap();
        std::fs::write(common.join("worktrees/a/commondir"), "../..\n").unwrap();
        std::fs::write(
            common.join("worktrees/b/commondir"),
            format!("{}\n", common.display()),
        )
        .unwrap();
        let mut main_process = process(10, 1, Some(100));
        main_process.cwd = Some(main.clone());
        main_process.uptime_seconds = Some(500);
        let mut a_process = process(11, 10, Some(200));
        a_process.cwd = Some(a.clone());
        a_process.uptime_seconds = Some(100);
        let mut b_process = process(12, 10, None);
        b_process.cwd = Some(b.clone());
        let mut clone_process = process(13, 1, Some(400));
        clone_process.cwd = Some(clone);
        let tree = build(vec![
            main_process.clone(),
            a_process.clone(),
            b_process.clone(),
            clone_process,
        ])
        .unwrap();
        let development = select(&tree, "group:projects").unwrap();
        assert_eq!(development.children.len(), 2);
        assert_eq!(tree.totals.processes, 4);
        assert_eq!(tree.totals.resident_bytes, 700);
        let repository_ref = stable_ref(
            "repository",
            &common.canonicalize().unwrap().to_string_lossy(),
        );
        let mut repository = select(&tree, &repository_ref).unwrap();
        assert_eq!(repository.name, "project");
        assert_eq!(repository.children.len(), 3);
        assert_eq!(repository.totals.processes, 3);
        assert_eq!(repository.totals.resident_bytes, 300);
        assert_eq!(repository.totals.memory_unknown, 1);
        assert_eq!(repository.totals.oldest_uptime_seconds, Some(500));
        assert_eq!(repository.totals.uptime_unknown, 1);
        let old_ref = stable_ref("development", &a.canonicalize().unwrap().to_string_lossy());
        let checkout = select(&tree, &old_ref).unwrap();
        assert_eq!(checkout.name, "worktree a");
        assert_eq!(checkout.totals.processes, 1);
        assert_eq!(checkout.children[0].reference, "pid:11");
        assert!(select(&tree, "pid:10").unwrap().children.is_empty());
        assert!(
            repository
                .children
                .iter()
                .any(|node| node.name == "main checkout")
        );
        let without_main = build(vec![b_process, a_process.clone()]).unwrap();
        assert_eq!(
            select(&without_main, &repository_ref).unwrap().name,
            "project"
        );
        assert_eq!(select(&without_main, &old_ref).unwrap().reference, old_ref);
        let single = build(vec![a_process]).unwrap();
        let single_group = select(&single, &old_ref).unwrap();
        assert_eq!(single_group.children[0].kind, "process");
        assert!(select(&single, &repository_ref).is_err());
        limit_depth(&mut repository, 0);
        assert_eq!(repository.omitted_children, 3);
        assert_eq!(repository.totals.resident_bytes, 300);
    }

    #[test]
    fn malformed_common_directory_keeps_checkout_separate() {
        let temp = tempfile::tempdir().unwrap();
        let main = temp.path().join("project");
        let worktree = temp.path().join("worktree");
        let admin = main.join(".git/worktrees/example");
        std::fs::create_dir_all(&admin).unwrap();
        std::fs::create_dir(&worktree).unwrap();
        std::fs::write(
            worktree.join(".git"),
            format!("gitdir: {}\n", admin.display()),
        )
        .unwrap();
        std::fs::write(admin.join("commondir"), "\n").unwrap();
        let mut a = process(10, 1, Some(100));
        a.cwd = Some(main);
        let mut b = process(11, 1, Some(200));
        b.cwd = Some(worktree.clone());
        let tree = build(vec![a, b]).unwrap();
        assert_eq!(select(&tree, "group:projects").unwrap().children.len(), 2);
        let reference = stable_ref(
            "development",
            &worktree.canonicalize().unwrap().to_string_lossy(),
        );
        assert!(
            select(&tree, &reference)
                .unwrap()
                .evidence
                .iter()
                .any(|reason| reason.starts_with("Repository metadata unavailable:"))
        );
    }

    #[test]
    fn outer_app_bundle_groups_helpers_without_name_only_matching() {
        let mut main = process(10, 1, Some(100));
        main.executable = Some("/Applications/Browser.app/Contents/MacOS/Browser".into());
        let mut helper = process(11, 10, Some(200));
        helper.executable = Some(
            "/Applications/Browser.app/Contents/Helpers/Helper.app/Contents/MacOS/Helper".into(),
        );
        let tree = build(vec![main, helper, process(12, 1, Some(300))]).unwrap();
        let app = select(&tree, "group:applications").unwrap();
        assert_eq!(app.totals.processes, 2);
        assert_eq!(app.children[0].name, "Browser");
        assert_eq!(tree.totals.processes, 3);
    }

    #[test]
    fn cycle_and_reused_parent_cannot_hide_or_duplicate_processes() {
        let mut a = process(10, 11, Some(10));
        let mut b = process(11, 10, Some(20));
        a.started_unix_us = Some(1);
        b.started_unix_us = Some(1);
        let tree = build(vec![a, b]).unwrap();
        assert_eq!(tree.totals.processes, 2);
        assert_eq!(tree.totals.resident_bytes, 30);
        let tree = build(vec![process(12, 1, Some(10)), process(10, 12, Some(20))]).unwrap();
        assert!(select(&tree, "pid:12").unwrap().children.is_empty());
        assert!(select(&tree, "pid:999").is_err());
    }

    #[test]
    fn duplicate_pid_is_rejected_instead_of_double_counted() {
        assert!(build(vec![process(10, 1, None), process(10, 1, None)]).is_err());
    }

    #[test]
    fn tool_names_do_not_establish_ownership() {
        for name in ["node", "python3.99", "herdr", "unfamiliar-tool"] {
            let mut item = process(10, 1, None);
            item.name = name.into();
            let tree = build(vec![item]).unwrap();
            assert_eq!(tree.children[0].reference, "group:unclassified", "{name}");
        }
    }

    #[test]
    fn relocated_and_cloned_app_bundles_share_structural_rules() {
        for path in [
            "/Volumes/Tools/Mystery.app/Contents/MacOS/run",
            "/tmp/clone/Mystery.app.bundle/Contents/MacOS/run",
            "/Applications/Docker.app/Contents/MacOS/run",
        ] {
            let mut item = process(10, 1, None);
            item.executable = Some(path.into());
            let tree = build(vec![item]).unwrap();
            assert_eq!(tree.children[0].reference, "group:applications", "{path}");
        }
        let mut item = process(10, 1, None);
        item.executable = Some("/tmp/notes.app/bin/tool".into());
        let tree = build(vec![item]).unwrap();
        assert_eq!(tree.children[0].reference, "group:unclassified");
    }

    #[test]
    fn ancestor_inference_requires_compatible_identity_and_cwd() {
        let directory = tempfile::tempdir().unwrap();
        let mut app = process(10, 1, Some(10));
        app.uid = Some(501);
        app.cwd = Some(directory.path().into());
        app.executable = Some("/Volumes/External/Invented.app/Contents/MacOS/start".into());
        let mut helper = process(11, 10, Some(20));
        helper.name = "unfamiliar-helper".into();
        helper.uid = app.uid;
        helper.cwd = app.cwd.clone();
        let mut grandchild = helper.clone();
        grandchild.pid = 12;
        grandchild.ppid = Some(11);
        grandchild.started_unix_us = Some(12);
        let tree = build(vec![grandchild.clone(), helper.clone(), app.clone()]).unwrap();
        assert_eq!(
            select(&tree, "group:applications")
                .unwrap()
                .totals
                .processes,
            3
        );
        assert!(
            select(&tree, "pid:12")
                .unwrap()
                .evidence
                .iter()
                .any(|reason| reason.starts_with("Inferred from ancestor pid:10"))
        );
        assert_eq!(tree.totals.resident_bytes, 50);
        let mut mismatched_user = helper.clone();
        mismatched_user.uid = Some(502);
        let mut missing_user = helper.clone();
        missing_user.uid = None;
        let mut missing_cwd = helper.clone();
        missing_cwd.cwd = None;
        let mut different_cwd = helper.clone();
        different_cwd.cwd = Some(directory.path().join("elsewhere"));
        let mut older_child = helper.clone();
        older_child.started_unix_us = Some(9);
        let mut missing_start = helper.clone();
        missing_start.started_unix_us = None;
        for child in [
            mismatched_user,
            missing_user,
            missing_cwd,
            different_cwd,
            older_child,
            missing_start,
        ] {
            let tree = build(vec![app.clone(), child]).unwrap();
            assert_eq!(
                select(&tree, "group:unclassified")
                    .unwrap()
                    .totals
                    .processes,
                1
            );
            assert!(select(&tree, "pid:10").unwrap().children.is_empty());
        }
        let mut own_app = helper.clone();
        own_app.executable = Some("/opt/Other.app/Contents/MacOS/start".into());
        let tree = build(vec![app.clone(), own_app]).unwrap();
        assert_eq!(
            select(&tree, "group:applications").unwrap().children.len(),
            2
        );
        std::fs::create_dir(directory.path().join(".git")).unwrap();
        let tree = build(vec![app, helper]).unwrap();
        assert_eq!(select(&tree, "group:projects").unwrap().totals.processes, 1);
        assert!(select(&tree, "pid:10").unwrap().children.is_empty());
        assert!(
            select(&tree, "pid:10")
                .unwrap()
                .evidence
                .iter()
                .any(|reason| reason.contains("project candidate"))
        );
    }

    #[test]
    fn unknown_ancestry_cycles_and_long_chains_stay_bounded() {
        let mut app = process(2, 1, None);
        app.executable = Some("/opt/Fictional.app/Contents/MacOS/start".into());
        app.uid = Some(501);
        app.cwd = Some("/".into());
        let mut processes = vec![app];
        for pid in 3..=132 {
            let mut child = process(pid, pid - 1, None);
            child.uid = Some(501);
            child.cwd = Some("/".into());
            processes.push(child);
        }
        let tree = build(processes).unwrap();
        assert_eq!(tree.totals.processes, 131);
        assert_eq!(
            select(&tree, "group:applications")
                .unwrap()
                .totals
                .processes,
            129
        );
        assert_eq!(
            select(&tree, "group:unclassified")
                .unwrap()
                .totals
                .processes,
            2
        );
    }
}
