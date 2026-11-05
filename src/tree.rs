use crate::model::{Attribution, AttributionSource, Diagnostic, Grouping, Node, Process, Totals};
use anyhow::{Result, bail};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    path::PathBuf,
};

mod applications;
mod executables;
#[cfg(test)]
mod project_tests;
mod projection;
#[cfg(test)]
pub(crate) mod versions;

use crate::extensions::host::{
    Facts,
    assignment::{self as assign, Assignment, system_executable},
};

pub(crate) use crate::extensions::host::stable_ref;

fn assignment_from_tag(
    tag: &crate::extensions::tags::Tag,
    source: &str,
    registry: &crate::extensions::placement::Registry,
) -> Assignment {
    if tag.dimension == "bin" {
        return if system_executable(tag.path.as_deref()) {
            assign::system("Effective system executable tag")
        } else {
            unknown_assignment()
        };
    }
    let declaration = &registry.ownership[&tag.dimension];
    let key = tag
        .assignment
        .as_ref()
        .and_then(|a| a.group_key.clone())
        .unwrap_or_else(|| {
            tag.path
                .as_deref()
                .map(|path| crate::extensions::tags::ownership_key(&tag.dimension, &tag.id, path))
                .unwrap_or_else(|| tag.id.clone())
        });
    let path_attribution = || {
        tag.path
            .as_ref()
            .filter(|_| matches!(tag.basis.as_str(), "cwd" | "executable"))
            .map(|path| {
                let mut attribution =
                    Attribution::new(AttributionSource::Extension, Some(path.clone()));
                attribution.extension = Some(crate::model::ExtensionAttribution {
                    provider: source.into(),
                    key: tag.id.clone(),
                    label: tag.label.clone(),
                    basis: tag.basis.clone(),
                });
                attribution
            })
    };
    if let Some(metadata) = &tag.assignment {
        let mut assignment = (**metadata).clone();
        assignment.key = if metadata.group_key.is_some() {
            metadata.key.clone()
        } else {
            key
        };
        assignment.name = tag.label.clone();
        if assignment
            .attribution
            .as_ref()
            .is_some_and(|a| matches!(a.source, AttributionSource::Extension))
        {
            if let Some(public) = path_attribution() {
                assignment.attribution.as_mut().unwrap().extension = public.extension;
            } else {
                assignment.attribution = None;
            }
            assignment.evidence = format!("Tag {} from {}", tag.id, source);
        }
        return assignment;
    }
    Assignment {
        group_key: None,
        category: declaration.category.clone(),
        category_name: registry.categories[&declaration.category].label.clone(),
        key,
        name: tag.label.clone(),
        evidence: format!("Tag {} from {}", tag.id, source),
        projection: tag.projection.as_deref().cloned().or_else(|| {
            (declaration.category != "applications" && declaration.category != "unclassified").then(
                || crate::extensions::placement::Projection {
                    grouping: declaration.grouping.clone(),
                    parents: vec![],
                },
            )
        }),
        attribution: path_attribution(),
    }
}

fn unknown_assignment() -> Assignment {
    assign::unknown("No effective ownership tag")
}

fn apply_identity_tags(process: &mut Process, report: &crate::extensions::tags::Report) {
    process.host_resource = report
        .effective
        .iter()
        .filter_map(|entry| entry.tag.host_resource.as_ref())
        .find(|resource| resource.link_key.is_some())
        .cloned()
        .or_else(|| {
            report
                .effective
                .iter()
                .find_map(|entry| entry.tag.host_resource.clone())
        });
    process.supplementary_labels = report
        .effective
        .iter()
        .filter(|entry| entry.tag.supplementary_to.is_some())
        .map(|entry| entry.tag.label.clone())
        .collect();
    let effective = |dimension: &str| {
        report
            .effective
            .iter()
            .find(|e| e.tag.dimension == dimension)
    };
    let named = effective("work")
        .map(|entry| (crate::model::NameReason::Work, entry))
        .or_else(|| effective("name").map(|entry| (crate::model::NameReason::Name, entry)));
    if let Some((reason, entry)) = named {
        process.original_name = Some(std::mem::replace(
            &mut process.name,
            entry.tag.label.clone(),
        ));
        process.name_source = Some(crate::model::NameSource {
            reason,
            source: entry.source.clone(),
            basis: entry.tag.basis.clone(),
        });
    }
    process.version = effective("version").and_then(|entry| {
        Some(crate::model::Version {
            subject: entry.tag.subject.clone()?,
            value: entry.tag.value.clone()?,
            basis: entry.tag.basis.clone(),
            source: entry.source.clone(),
        })
    });
}

pub fn build(processes: Vec<Process>) -> Result<Node> {
    let settings = crate::extensions::default_settings()?;
    let cancel = crate::snapshot::Cancellation::default();
    build_with_context(
        processes,
        &crate::extensions::Context::new(&settings, &cancel),
    )
}

pub(crate) fn build_with_context(
    processes: Vec<Process>,
    context: &crate::extensions::Context<'_>,
) -> Result<Node> {
    build_with_providers(
        processes,
        context,
        &crate::extensions::default_classifiers(),
    )
}

pub fn build_with_providers(
    processes: Vec<Process>,
    context: &crate::extensions::Context<'_>,
    providers: &[&dyn crate::extensions::Extension],
) -> Result<Node> {
    let mut facts = Facts::new(&processes);
    let mut all_providers: Vec<&dyn crate::extensions::Extension> =
        crate::core::classifiers().to_vec();
    all_providers.extend(crate::extensions::round_classifiers());
    let mut provider_ids: HashSet<_> = all_providers.iter().map(|p| p.descriptor().id).collect();
    for provider in providers.iter().copied() {
        let id = crate::extensions::boundary(|| Ok(provider.descriptor().id));
        if id.is_ok_and(|id| !provider_ids.insert(id)) {
            continue;
        }
        all_providers.push(provider);
    }
    let registry = crate::extensions::placement::Registry::new(&all_providers, context);
    let (mut reports, diagnostics) =
        crate::extensions::tags::classify(&all_providers, context, &facts);
    let executables = std::mem::take(&mut facts.executables);
    let bundle_metadata = std::mem::take(&mut facts.bundles);
    let mut seen = HashSet::new();
    let mut buckets: BTreeMap<String, (Assignment, Vec<Process>)> = BTreeMap::new();
    let mut evidence = HashMap::new();
    let mut attributions = HashMap::new();
    let mut assignments = HashMap::new();
    let processes: Vec<_> = processes
        .into_iter()
        .map(|mut process| {
            process.roles.clear();
            let report = reports.remove(&process.pid).unwrap_or_default();
            apply_identity_tags(&mut process, &report);
            process.tag_report = Some(report);
            process
        })
        .collect();
    for process in &processes {
        if !seen.insert(process.pid) {
            bail!("duplicate PID {} in snapshot", process.pid);
        }
        let report = process.tag_report.as_ref().expect("tag report");
        let selected = registry.selected(report, context.settings);
        let mut assignment = selected
            .map(|entry| assignment_from_tag(&entry.tag, &entry.source, &registry))
            .unwrap_or_else(unknown_assignment);
        if assignment.category == "unclassified"
            && let Some(entry) = selected
            && entry.tag.path.is_none()
            && let Some(path) = executables.get(&process.pid)
        {
            assignment.key =
                crate::extensions::tags::ownership_key(&entry.tag.dimension, &entry.tag.id, path);
        }
        let mut reasons = vec![assignment.evidence.clone()];
        if assignment.category == "applications" {
            reasons.extend(
                report
                    .effective
                    .iter()
                    .filter(|e| {
                        registry.ownership.get(&e.tag.dimension).is_some_and(|o| {
                            o.category != "applications" && o.category != "unclassified"
                        })
                    })
                    .filter_map(|e| {
                        e.tag.path.as_ref().map(|path| {
                            format!(
                                "Working-directory {} candidate: {}; application grouping retained",
                                e.tag.dimension,
                                path.display()
                            )
                        })
                    }),
            );
        }
        evidence.insert(process.pid, reasons);
        attributions.insert(process.pid, assignment.attribution.clone());
        assignments.insert(process.pid, assignment);
    }
    for process in processes {
        let assignment = assignments
            .remove(&process.pid)
            .expect("assignment for every process");
        let reference = stable_ref(&assignment.category, &assignment.key);
        let bucket = buckets
            .entry(reference)
            .or_insert_with(|| (assignment.clone(), Vec::new()));
        if bucket.0.key != assignment.key || bucket.0.category != assignment.category {
            bail!("group identity collision");
        }
        bucket.1.push(process);
    }
    let mut categories = BTreeMap::new();
    let mut projected: BTreeMap<String, Vec<_>> = BTreeMap::new();
    let mut bundles = Vec::new();
    for (reference, (assignment, processes)) in buckets {
        let grouping = assignment
            .projection
            .as_ref()
            .map(|p| p.grouping.clone())
            .unwrap_or_else(|| match assignment.category.as_str() {
                "applications" => Grouping::App,
                "system" => Grouping::System,
                _ => Grouping::Unknown,
            });
        let mut group = Node::group(
            reference,
            assignment.name,
            vec![assignment.evidence],
            grouping,
        );
        group.attribution = assignment.attribution;
        if let Some(attribution) = &mut group.attribution {
            attribution.via_pid = None;
            attribution.inferred = false;
        }
        group.children = process_forest(processes, &mut evidence);
        for child in &group.children {
            group.totals.add(&child.totals);
        }
        let category = categories
            .entry(assignment.category.clone())
            .or_insert_with(|| {
                Node::group(
                    crate::extensions::placement::category_ref(&assignment.category),
                    assignment.category_name,
                    Vec::new(),
                    Grouping::Category,
                )
            });
        category.totals.add(&group.totals);
        if let Some(projection) = assignment.projection {
            projected
                .entry(assignment.category.clone())
                .or_default()
                .push((group, projection.parents));
        } else if assignment.category == "applications" {
            bundles.push((
                group
                    .attribution
                    .as_ref()
                    .and_then(|a| a.path.clone())
                    .unwrap_or_else(|| PathBuf::from(assignment.key)),
                group,
            ));
        } else if assignment.category == "unclassified" {
            category
                .children
                .extend(executables::group(group, &registry, context.settings));
        } else if assignment.category == "system" {
            let mut services = Node::group(
                group.reference.clone(),
                group.name.clone(),
                group.evidence.clone(),
                Grouping::System,
            );
            services.totals = group.totals.clone();
            services.children = executables::group(group, &registry, context.settings);
            category.children.push(services);
        } else {
            category.children.push(group);
        }
    }
    for (id, groups) in projected {
        categories
            .get_mut(&id)
            .expect("projected category")
            .children
            .extend(projection::group(groups));
    }
    if let Some(applications) = categories.get_mut("applications") {
        applications.children.extend(applications::group(
            bundles,
            &bundle_metadata,
            context
                .settings
                .get("core.bundle_version")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true),
        ));
    }
    let mut root = Node::group(
        "group:all".into(),
        "Resources".into(),
        Vec::new(),
        Grouping::Category,
    );
    root.diagnostics = diagnostics;
    root.children = categories
        .into_iter()
        .map(|(id, mut node)| {
            node.category_order = registry.categories.get(&id).map(|c| c.order);
            node
        })
        .collect();
    for category in &root.children {
        root.totals.add(&category.totals);
    }
    let mut pending = vec![&mut root];
    while let Some(node) = pending.pop() {
        if let Some(process) = &node.process {
            node.attribution = attributions.remove(&process.pid).flatten();
        }
        pending.extend(node.children.iter_mut());
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
            .then(a.category_order.cmp(&b.category_order))
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
        if let Some(node) = process_node(pid, &mut by_pid, &children, evidence) {
            roots.push(node);
        }
    }
    while let Some(pid) = by_pid.keys().next().copied() {
        if let Some(mut node) = process_node(pid, &mut by_pid, &children, evidence) {
            node.evidence
                .push("Parent chain cycle or depth limit; displayed as a separate root".into());
            node.diagnostics.push(Diagnostic::new(
                "parent_chain_incomplete",
                "parent_chain",
                "Parent chain cycle or depth limit; displayed as a separate root",
            ));
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
) -> Option<Node> {
    let mut built: Vec<(Node, Option<usize>)> = Vec::new();
    let mut pending = vec![(pid, None, 0usize)];
    while let Some((pid, parent, depth)) = pending.pop() {
        let Some(process) = processes.remove(&pid) else {
            continue;
        };
        let index = built.len();
        built.push((process_leaf(pid, process, evidence), parent));
        if depth < 128 {
            for child in children.get(&Some(pid)).into_iter().flatten().rev() {
                pending.push((*child, Some(index), depth + 1));
            }
        }
    }
    while let Some((mut node, parent)) = built.pop() {
        node.children.reverse();
        let Some(parent) = parent else {
            return Some(node);
        };
        built[parent].0.totals.add(&node.totals);
        built[parent].0.children.push(node);
    }
    None
}

fn process_leaf(pid: u32, process: Process, evidence: &mut HashMap<u32, Vec<String>>) -> Node {
    let mut totals = Totals::default();
    totals.add_process(&process);
    Node {
        host_resources: vec![],
        category_order: None,
        reference: format!("pid:{pid}"),
        name: process.name.clone(),
        kind: "process",
        grouping: None,
        evidence: evidence.remove(&pid).unwrap_or_default(),
        attribution: None,
        diagnostics: Vec::new(),
        process: Some(Box::new(process)),
        resource: None,
        container: None,
        runtime: None,
        container_totals: None,
        totals,
        children: Vec::new(),
        omitted_children: 0,
        version: None,
    }
}

pub(crate) fn compose_category(root: &mut Node, mut incoming: Node) -> Result<()> {
    validate_references(&incoming)?;
    let index = root
        .children
        .iter()
        .position(|node| node.reference == incoming.reference);
    if let Some(index) = index {
        let current = &root.children[index];
        if current.kind != "group"
            || incoming.kind != "group"
            || current.grouping != Some(Grouping::Category)
            || incoming.grouping != Some(Grouping::Category)
            || current.name != incoming.name
            || (current.container_totals.is_some() && incoming.container_totals.is_some())
        {
            bail!("incompatible_category_composition");
        }
    }
    let mut references = HashSet::new();
    let mut pending = vec![&*root];
    while let Some(node) = pending.pop() {
        references.insert(node.reference.as_str());
        pending.extend(&node.children);
    }
    let mut pending = vec![&incoming];
    while let Some(node) = pending.pop() {
        if references.contains(node.reference.as_str())
            && !(index.is_some() && std::ptr::eq(node, &incoming))
        {
            bail!("colliding_category_content");
        }
        pending.extend(&node.children);
    }
    root.totals.add(&incoming.totals);
    if let Some(index) = index {
        let current = &mut root.children[index];
        current.totals.add(&incoming.totals);
        if incoming.container_totals.is_some() {
            current.container_totals = incoming.container_totals.take();
        }
        current.evidence.extend(
            incoming
                .evidence
                .into_iter()
                .map(|value| format!("Collected scope: {value}")),
        );
        current.diagnostics.extend(incoming.diagnostics);
        current.children.extend(incoming.children);
    } else {
        root.children.push(incoming);
    }
    Ok(())
}

pub fn select(root: &Node, reference: &str) -> Result<Node> {
    let mut pending = vec![(root, Vec::new(), Vec::new())];
    while let Some((node, context, diagnostics)) = pending.pop() {
        if node.reference == reference {
            let mut selected = node.clone();
            selected.evidence.extend(context);
            selected.diagnostics.extend(diagnostics);
            return Ok(selected);
        }
        let mut context = context;
        let mut diagnostics = diagnostics;
        diagnostics.extend(
            node.diagnostics
                .iter()
                .filter(|diagnostic| !diagnostic.is_process_details())
                .cloned(),
        );
        if node.kind == "group" && node.reference != "group:all" {
            context.push(format!("Grouped under: {} ({})", node.name, node.reference));
            context.extend(node.evidence.clone());
            if let Some(runtime) = &node.runtime {
                diagnostics.extend(runtime.unavailable.clone());
                context.extend(
                    runtime
                        .unavailable
                        .iter()
                        .map(|issue| format!("Runtime unavailable: {issue}")),
                );
            }
        }
        pending.extend(
            node.children
                .iter()
                .map(|child| (child, context.clone(), diagnostics.clone())),
        );
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

fn host_scope(node: &Node) -> Option<&str> {
    node.runtime
        .as_ref()
        .and_then(|r| r.host_scope.as_deref())
        .or_else(|| {
            node.container
                .as_ref()
                .and_then(|c| c.host_scope.as_deref())
        })
}

pub(crate) fn link_host_resources(root: &mut Node) {
    let mut usages: BTreeMap<String, Vec<crate::model::HostUsage>> = BTreeMap::new();
    let mut configurations = BTreeMap::new();
    let mut pending = vec![&*root];
    while let Some(node) = pending.pop() {
        if let Some(scope) = host_scope(node) {
            let configuration = node
                .runtime
                .as_ref()
                .and_then(|r| r.configuration.clone())
                .or_else(|| {
                    node.container
                        .as_ref()
                        .and_then(|c| c.configuration.clone())
                });
            if let Some(configuration) = configuration {
                configurations.insert(scope.to_owned(), configuration);
            }
        }
        if let Some(process) = &node.process
            && let Some(resource) = &process.host_resource
            && let Some(scope) = &resource.link_key
            && let Some(identity) = crate::extensions::ProcessIdentity::observed(process)
        {
            usages
                .entry(scope.clone())
                .or_default()
                .push(crate::model::HostUsage {
                    reference: node.reference.clone(),
                    identity,
                    cpu_percent: process.cpu_percent,
                    resident_bytes: process.resident_bytes,
                });
        }
        pending.extend(&node.children);
    }
    let mut pending = vec![root];
    while let Some(node) = pending.pop() {
        node.host_resources = host_scope(node)
            .and_then(|scope| usages.get(scope))
            .cloned()
            .unwrap_or_default();
        if let Some(resource) = node.process.as_mut().and_then(|p| p.host_resource.as_mut())
            && let Some(key) = &resource.link_key
        {
            resource.configuration = configurations.get(key).cloned();
        }
        pending.extend(&mut node.children);
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
        let attribution = serde_json::to_value(&repository.attribution).unwrap();
        assert_eq!(attribution["source"], "git_common_dir");
        assert_eq!(
            attribution["path"],
            common.canonicalize().unwrap().to_str().unwrap()
        );
        assert_eq!(repository.children.len(), 3);
        assert_eq!(repository.totals.processes, 3);
        assert_eq!(repository.totals.resident_bytes, 300);
        assert_eq!(repository.totals.memory_unknown, 1);
        assert_eq!(repository.totals.oldest_uptime_seconds, Some(500));
        assert_eq!(repository.totals.uptime_unknown, 1);
        let old_ref = crate::extensions::tags::ownership_ref(
            "project",
            &crate::extensions::tags::path_id("project", &a),
            &a,
        );
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
        let reference = crate::extensions::tags::ownership_ref(
            "project",
            &crate::extensions::tags::path_id("project", &worktree),
            &worktree,
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
    fn round_facts_are_computed_once_for_every_builtin_source() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir(temp.path().join(".git")).unwrap();
        let mut processes = Vec::new();
        for pid in [10, 11, 12] {
            let mut item = process(pid, 1, Some(100));
            item.uid = Some(501);
            item.cwd = Some(temp.path().into());
            item.executable = Some("/opt/tool/run".into());
            processes.push(item);
        }
        processes[2].executable = Some("/Applications/Demo.app/Contents/MacOS/main".into());
        let before = crate::extensions::host::facts::FACTS_BUILT.with(std::cell::Cell::get);
        let tree = build(processes).unwrap();
        assert_eq!(
            crate::extensions::host::facts::FACTS_BUILT.with(std::cell::Cell::get) - before,
            1
        );
        assert_eq!(select(&tree, "group:projects").unwrap().totals.processes, 2);
        assert_eq!(
            select(&tree, "group:applications")
                .unwrap()
                .totals
                .processes,
            1
        );
        let report = select(&tree, "pid:10")
            .unwrap()
            .process
            .unwrap()
            .tag_report
            .unwrap();
        for source in ["project", "bin"] {
            assert!(
                report.effective.iter().any(|e| e.source == source),
                "{source}"
            );
        }
        temp.close().unwrap();
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
    fn deep_parent_chains_build_on_a_half_size_thread_stack() {
        let processes: Vec<_> = (2..=200)
            .map(|pid| {
                let mut child = process(pid, pid - 1, Some(1));
                child.uid = Some(501);
                child.cwd = Some("/".into());
                child
            })
            .collect();
        let tree = std::thread::Builder::new()
            .stack_size(1024 * 1024)
            .spawn(move || {
                let tree = build(processes).unwrap();
                assert_eq!(select(&tree, "pid:200").unwrap().totals.processes, 1);
                assert!(serde_json::to_string(&tree).unwrap().contains("pid:200"));
                let markdown = crate::output::markdown(&crate::model::Snapshot {
                    schema: crate::model::JSON_SCHEMA_VERSION,
                    extensions: vec![],
                    captured_unix_ms: 1,
                    sample_ms: 1,
                    collection_ms: 1,
                    system: Default::default(),
                    scope: "group:all".into(),
                    warnings: vec![],
                    root: tree.clone(),
                });
                assert!(markdown.contains("pid:2 "));
                tree.totals.processes
            })
            .unwrap()
            .join()
            .unwrap();
        assert_eq!(tree, 199);
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
