use super::*;
use std::fs;
use std::path::Path;

fn process(pid: u32, cwd: &Path) -> Process {
    Process {
        pid,
        ppid: Some(1),
        uid: Some(501),
        started_unix_us: Some(u64::from(pid)),
        uptime_seconds: Some(1000 - u64::from(pid)),
        cwd: Some(cwd.to_path_buf()),
        name: "unknown-tool".into(),
        resident_bytes: Some(100),
        cpu_percent: Some(1.0),
        ..Process::default()
    }
}

fn manifest(root: &Path, name: &str) {
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(root.join(name), "").unwrap();
}

fn reference(namespace: &str, root: &Path) -> String {
    if namespace == "repository" {
        return stable_ref(namespace, &root.canonicalize().unwrap().to_string_lossy());
    }
    crate::extensions::tags::ownership_ref(
        "project",
        &crate::extensions::tags::path_id("project", root),
        root,
    )
}

#[test]
fn manifest_projects_need_no_git_and_same_names_remain_separate() {
    let temp = tempfile::tempdir().unwrap();
    let markers = [
        "Cargo.toml",
        "package.json",
        "pyproject.toml",
        "pom.xml",
        "settings.gradle.kts",
        "settings.gradle",
        "build.gradle.kts",
        "build.gradle",
        "go.work",
        "go.mod",
        "build.sbt",
        "Gemfile",
        "composer.json",
        "pubspec.yaml",
        "mix.exs",
        "Package.swift",
        "deno.json",
        "deno.jsonc",
        "CMakeLists.txt",
        "meson.build",
        "build.zig",
    ];
    let mut processes = Vec::new();
    for (index, marker) in markers.iter().enumerate() {
        let path = temp.path().join(format!("{index}/example"));
        manifest(&path, marker);
        processes.push(process(index as u32 + 10, &path.join("src")));
    }
    let tree = build(processes.clone()).unwrap();
    let category = select(&tree, "group:projects").unwrap();
    assert_eq!(category.name, "Projects");
    assert_eq!(category.children.len(), markers.len());
    assert_eq!(category.totals.processes, markers.len());
    assert_eq!(category.totals.resident_bytes, markers.len() as u64 * 100);
    for (index, marker) in markers.iter().enumerate() {
        let path = temp.path().join(format!("{index}/example"));
        let group = select(&tree, &reference("project", &path)).unwrap();
        assert_eq!(group.name, "example");
        assert_eq!(group.children[0].reference, format!("pid:{}", index + 10));
        assert!(group.evidence.contains(&format!(
            "project {marker} {}",
            path.canonicalize().unwrap().display()
        )));
    }
    processes.reverse();
    let reversed = build(processes).unwrap();
    assert_eq!(
        serde_json::to_value(tree).unwrap(),
        serde_json::to_value(reversed).unwrap()
    );
    temp.close().unwrap();
}

#[test]
fn nearest_manifest_stops_at_git_boundaries_and_keeps_unknown_cwd() {
    let temp = tempfile::tempdir().unwrap();
    let outer = temp.path().join("outer");
    let inner = outer.join("inner");
    let repository = inner.join("repo");
    manifest(&outer, "package.json");
    manifest(&inner, "Cargo.toml");
    fs::create_dir_all(repository.join(".git")).unwrap();
    fs::create_dir_all(repository.join("src")).unwrap();
    let unknown = temp.path().join("plain");
    fs::create_dir_all(unknown.join("package.json")).unwrap();
    let missing = temp.path().join("missing");
    let mut app = process(15, &inner);
    app.executable = Some("/Applications/Example.app/Contents/MacOS/example".into());
    let tree = build(vec![
        process(10, &inner.join("src")),
        process(11, &outer.join("src")),
        process(12, &repository.join("src")),
        process(13, &unknown),
        process(14, &missing),
        app,
    ])
    .unwrap();
    assert_eq!(select(&tree, "group:projects").unwrap().children.len(), 3);
    assert_eq!(
        select(&tree, &reference("project", &inner))
            .unwrap()
            .children[0]
            .reference,
        "pid:10"
    );
    let repo = select(&tree, &reference("development", &repository)).unwrap();
    assert_eq!(repo.children[0].reference, "pid:12");
    assert_eq!(
        select(&tree, "group:unclassified")
            .unwrap()
            .totals
            .processes,
        2
    );
    assert_eq!(
        select(&tree, "pid:13").unwrap().process.unwrap().cwd,
        Some(unknown)
    );
    assert_eq!(
        select(&tree, "group:applications")
            .unwrap()
            .totals
            .processes,
        1
    );
    assert!(
        select(&tree, "pid:15")
            .unwrap()
            .evidence
            .iter()
            .any(|e| e.contains("Working-directory project candidate:"))
    );
    temp.close().unwrap();
}

#[test]
fn checkout_projects_preserve_repository_structure_and_count_each_process_once() {
    let temp = tempfile::tempdir().unwrap();
    let main = temp.path().join("example");
    let worktree = temp.path().join("other");
    let admin = main.join(".git/worktrees/branch");
    fs::create_dir_all(&admin).unwrap();
    fs::create_dir_all(&worktree).unwrap();
    fs::write(
        worktree.join(".git"),
        format!("gitdir: {}", admin.display()),
    )
    .unwrap();
    fs::write(admin.join("commondir"), "../..").unwrap();
    manifest(&main, "package.json");
    let service = main.join("services/api");
    let other_service = worktree.join("services/api");
    manifest(&service, "Cargo.toml");
    manifest(&other_service, "pyproject.toml");
    let parent = process(10, &main);
    let mut child = process(11, &service.join("src"));
    child.ppid = Some(10);
    let mut helper = process(12, &service);
    helper.ppid = Some(11);
    helper.resident_bytes = None;
    let checkout_process = process(13, &other_service);
    let tree = build(vec![parent, child.clone(), helper, checkout_process]).unwrap();
    let repo_ref = reference("repository", &main.join(".git"));
    let repo = select(&tree, &repo_ref).unwrap();
    assert_eq!(repo.children.len(), 2);
    assert_eq!(repo.totals.processes, 4);
    assert_eq!(repo.totals.resident_bytes, 300);
    assert_eq!(repo.totals.memory_unknown, 1);
    assert_eq!(repo.totals.oldest_uptime_seconds, Some(990));
    let checkout_ref = reference("development", &main);
    let checkout = select(&tree, &checkout_ref).unwrap();
    assert_eq!(checkout.name, "main checkout");
    assert_eq!(checkout.children.len(), 2);
    assert_eq!(checkout.totals.processes, 3);
    let service_ref = reference("project", &service);
    let mut service_group = select(&tree, &service_ref).unwrap();
    assert_eq!(service_group.children[0].reference, "pid:11");
    assert_eq!(service_group.children[0].children[0].reference, "pid:12");
    assert_eq!(
        service_group.children[0].process.as_ref().unwrap().ppid,
        Some(10)
    );
    limit_depth(&mut service_group, 0);
    assert_eq!(service_group.omitted_children, 1);
    assert_eq!(service_group.totals.processes, 2);
    assert_eq!(tree.totals.processes, 4);
    let single = build(vec![child]).unwrap();
    assert!(select(&single, &repo_ref).is_err());
    let checkout = select(&single, &checkout_ref).unwrap();
    assert_eq!(checkout.children[0].reference, service_ref);
    assert_eq!(checkout.totals.processes, 1);
    temp.close().unwrap();
}
