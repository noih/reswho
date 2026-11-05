use crate::extensions::{
    Capability, ClassificationProposal, Context, Contribution, ProcessIdentity, Request,
    host::{
        Facts, Module, files,
        proposals::{identity_proposal, label, with_id},
        runtime::{Argv, Shared, entries_enabled, entry_definition, resolve, suspicious},
        sanitize_command, stable_ref,
    },
};
use crate::settings::Definition;
use std::{
    collections::HashMap,
    path::{Component, Path, PathBuf},
};

pub(crate) const ID: &str = "node";

pub(crate) static MODULE: Module = Module {
    registration,
    decorate: None,
    id: ID,
    capability: Capability::Classify,
    execute,
    settings,
};

fn settings() -> anyhow::Result<Vec<Definition>> {
    Ok(vec![entry_definition(ID)?])
}

fn execute(context: &Context<'_>, request: Request<'_>) -> anyhow::Result<Contribution> {
    let Request::Classify(facts) = request else {
        anyhow::bail!("unsupported extension request");
    };
    Ok(Contribution::Classify {
        proposals: proposals(
            facts,
            entries_enabled(context.settings, ID),
            &mut Shared {
                provider: ID,
                reader: &context.argv,
            },
        ),
    })
}

const GENERIC_STEMS: &[&str] = &["index", "main", "server", "cli", "app", "start"];
const BLOCKED_OPTIONS: &[&str] = &[
    "-e",
    "--eval",
    "-p",
    "--print",
    "-r",
    "--require",
    "--import",
    "--loader",
    "--experimental-loader",
    "-i",
    "--interactive",
    "-c",
    "--check",
    "--test",
    "--run",
    "--input-type",
    "--watch",
];
const VALUE_OPTIONS: &[&str] = &[
    "-C",
    "--conditions",
    "--title",
    "--env-file",
    "--inspect-port",
];
const FLAG_OPTIONS: &[&str] = &[
    "--inspect",
    "--inspect-brk",
    "--inspect-wait",
    "--enable-source-maps",
    "--expose-gc",
    "--preserve-symlinks",
    "--preserve-symlinks-main",
    "--abort-on-uncaught-exception",
    "--pending-deprecation",
    "--throw-deprecation",
    "--no-warnings",
    "--no-deprecation",
    "--trace-warnings",
    "--trace-deprecation",
    "--trace-uncaught",
    "--liftoff-only",
];
const SCRIPT_EXTENSIONS: &[&str] = &["js", "mjs", "cjs"];

fn node_executable(executable: &Path) -> bool {
    executable
        .file_name()
        .is_some_and(|name| name == "node" || name == "nodejs")
}

pub(crate) type Versions = HashMap<(PathBuf, Option<u32>), Option<String>>;

pub(crate) fn header_version(
    executable: &Path,
    owner: Option<u32>,
    cache: &mut Versions,
) -> Option<String> {
    let bin = executable.parent()?;
    if bin.file_name()? != "bin" {
        return None;
    }
    let prefix = bin.parent()?;
    cache
        .entry((prefix.to_path_buf(), owner))
        .or_insert_with(|| {
            let header = prefix.join("include/node/node_version.h");
            let files::Trusted::Bytes(bytes) = files::read(&header, owner) else {
                return None;
            };
            let text = String::from_utf8(bytes).ok()?;
            let part = |name: &str| {
                files::define(&text, name)
                    .filter(|value| (1..=9).contains(&value.len()))
                    .filter(|value| value.bytes().all(|byte| byte.is_ascii_digit()))
            };
            let value = format!(
                "{}.{}.{}",
                part("NODE_MAJOR_VERSION")?,
                part("NODE_MINOR_VERSION")?,
                part("NODE_PATCH_VERSION")?
            );
            files::version_text(&value).then_some(value)
        })
        .clone()
}

fn script(value: &str, cwd: Option<&Path>) -> Option<PathBuf> {
    let path = resolve(value, cwd)?;
    (path.is_file()
        && path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| SCRIPT_EXTENSIONS.contains(&e)))
    .then_some(path)
}

pub(crate) fn entry(argv: Vec<String>, executable: &Path, cwd: Option<&Path>) -> Option<PathBuf> {
    let command = sanitize_command(argv, Some(executable));
    let argv = command.argv.as_ref()?;
    let usable = |index: usize| !command.redacted_indices.contains(&index);
    let mut index = 1;
    while let Some(arg) = argv.get(index) {
        if arg == "--" {
            index += 1;
            break;
        }
        if !arg.starts_with('-') || arg == "-" {
            break;
        }
        let option = arg.split_once('=').map_or(arg.as_str(), |(key, _)| key);
        if BLOCKED_OPTIONS.contains(&option) {
            return None;
        }
        if arg.starts_with("--") && arg.contains('=') || FLAG_OPTIONS.contains(&arg.as_str()) {
            index += 1;
        } else if VALUE_OPTIONS.contains(&arg.as_str()) {
            index += 2;
        } else {
            return argv
                .get(index + 1)
                .filter(|_| usable(index + 1))
                .and_then(|next| script(next, cwd));
        }
    }
    let value = argv.get(index)?;
    if value.is_empty() || value == "-" || value == "inspect" || !usable(index) {
        return None;
    }
    resolve(value, cwd).filter(|path| path.is_file() || path.is_dir())
}

fn package_from_modules(path: &Path) -> Option<String> {
    let parts: Vec<_> = path
        .components()
        .filter_map(|c| match c {
            Component::Normal(value) => value.to_str(),
            _ => None,
        })
        .collect();
    let at = parts.iter().rposition(|part| *part == "node_modules")?;
    let first = *parts.get(at + 1)?;
    let package = if first.starts_with('@') {
        format!("{first}/{}", parts.get(at + 2)?)
    } else {
        first.to_owned()
    };
    (!first.starts_with('.') && parts.len() > at + 2 + usize::from(first.starts_with('@')))
        .then_some(package)
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Package {
    Missing,
    Unnamed,
    Named(String),
}

pub(crate) type Packages = HashMap<(PathBuf, u32), Package>;

fn package_json(directory: &Path, owner: u32, cache: &mut Packages) -> Option<String> {
    for directory in directory.ancestors().take(16) {
        let found = cache
            .entry((directory.to_path_buf(), owner))
            .or_insert_with(|| read_package(&directory.join("package.json"), owner));
        match found {
            Package::Missing => continue,
            Package::Unnamed => return None,
            Package::Named(name) => return Some(name.clone()),
        }
    }
    None
}

fn read_package(file: &Path, owner: u32) -> Package {
    match files::read(file, Some(owner)) {
        files::Trusted::Missing => Package::Missing,
        files::Trusted::Rejected => Package::Unnamed,
        files::Trusted::Bytes(bytes) => serde_json::from_slice::<serde_json::Value>(&bytes)
            .ok()
            .and_then(|value| value["name"].as_str().map(str::to_owned))
            .filter(|name| {
                !name.is_empty() && name.len() <= 128 && !name.chars().any(char::is_control)
            })
            .map_or(Package::Unnamed, Package::Named),
    }
}

pub(crate) fn work(
    path: &Path,
    owner: u32,
    cache: &mut Packages,
) -> Option<(String, String, &'static str)> {
    let (label, kind) = if let Some(package) = package_from_modules(path) {
        (package, "argv_package")
    } else if path.is_dir() {
        (package_json(path, owner, cache)?, "argv_package_json")
    } else if let Some(package) = path
        .parent()
        .and_then(|dir| package_json(dir, owner, cache))
    {
        (package, "argv_package_json")
    } else {
        let stem = path.file_stem()?.to_str()?;
        if GENERIC_STEMS.contains(&stem.to_ascii_lowercase().as_str()) || suspicious(stem) {
            return None;
        }
        (stem.to_owned(), "argv_script")
    };
    let id = format!(
        "work:{}",
        stable_ref("work", &path.to_string_lossy()).trim_start_matches("group:")
    );
    Some((id, label, kind))
}

pub(crate) fn proposals(
    facts: &Facts<'_>,
    entries: bool,
    argv: &mut dyn Argv,
) -> Vec<ClassificationProposal> {
    let user = unsafe { libc::geteuid() };
    let mut cache = Packages::new();
    let mut versions = Versions::new();
    let mut proposals = Vec::new();
    for process in facts.processes {
        let Some(identity) = ProcessIdentity::observed(process) else {
            continue;
        };
        let Some(executable) = facts.executables.get(&process.pid) else {
            continue;
        };
        if !node_executable(executable) {
            continue;
        }
        proposals.push(with_id(
            identity_proposal(identity, ID, "executable", "runtime", ID, None),
            "runtime:node".into(),
        ));
        if let Some(version) = header_version(executable, process.uid, &mut versions) {
            proposals.push(identity_proposal(
                identity,
                ID,
                "executable",
                "version",
                &version,
                Some(ID),
            ));
        }
        if !entries || process.uid != Some(user) {
            continue;
        }
        let args = match process.command.as_ref().and_then(|c| c.argv.clone()) {
            Some(args) => Some(args),
            None => argv.read(process),
        };
        let Some((id, name, kind)) = args
            .and_then(|args| entry(args, executable, process.cwd.as_deref()))
            .and_then(|entry| work(&entry, user, &mut cache))
        else {
            continue;
        };
        proposals.push(with_id(
            identity_proposal(identity, ID, kind, "work", &label(&name), None),
            id,
        ));
    }
    proposals
}

fn registration() -> crate::extensions::placement::Registration {
    use crate::extensions::placement::*;
    Registration {
        categories: vec![],
        ownership: vec![Ownership {
            grouping: crate::model::Grouping::Work,
            ..Ownership::new("work", "unclassified", 300)
        }],
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::extensions::host::runtime::entry_setting;
    use crate::model::Process;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| (*v).to_owned()).collect()
    }

    pub(crate) fn install(prefix: &Path, version: &str) -> PathBuf {
        let mut parts = version.split('.');
        let (major, minor, patch) = (
            parts.next().unwrap(),
            parts.next().unwrap(),
            parts.next().unwrap(),
        );
        let include = prefix.join("include/node");
        std::fs::create_dir_all(&include).unwrap();
        std::fs::write(
            include.join("node_version.h"),
            format!(
                "#ifndef SRC_NODE_VERSION_H_\n#define SRC_NODE_VERSION_H_\n\n#define NODE_MAJOR_VERSION {major}\n#define NODE_MINOR_VERSION {minor}\n#define NODE_PATCH_VERSION {patch}\n\n#define NODE_VERSION_IS_LTS 0\n#define NODE_VERSION_LTS_CODENAME \"\"\n\n#define NODE_VERSION_IS_RELEASE 1\n"
            ),
        )
        .unwrap();
        let binary = prefix.join("bin/node");
        std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
        std::fs::write(&binary, "").unwrap();
        binary
    }

    fn observed(executable: &Path, uid: Option<u32>) -> Vec<(String, Option<String>)> {
        let processes = [Process {
            pid: 1,
            ppid: Some(0),
            uid,
            started_unix_us: Some(1),
            name: "node".into(),
            executable: Some(executable.into()),
            ..Default::default()
        }];
        proposals(&Facts::new(&processes), false, &mut Counting(0, None))
            .into_iter()
            .filter_map(|p| match p.evidence {
                crate::extensions::ClassificationEvidence::Tags { proposal } => {
                    match proposal.operation {
                        crate::extensions::tags::Operation::Add { tag } => {
                            Some((tag.dimension, tag.value))
                        }
                        _ => None,
                    }
                }
                _ => None,
            })
            .collect()
    }

    fn version_of(executable: &Path, uid: Option<u32>) -> Option<String> {
        observed(executable, uid)
            .into_iter()
            .find(|(dimension, _)| dimension == "version")
            .and_then(|(_, value)| value)
    }

    #[test]
    fn node_versions_come_from_the_install_prefix_header_in_every_layout() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let user = Some(unsafe { libc::geteuid() });
        for (name, prefix, version) in [
            ("official tarball", "node-v24.18.0-darwin-arm64", "24.18.0"),
            (".pkg installer", "usr/local", "22.11.0"),
            (
                "nvm with NVM_DIR",
                "work/nvm-home/versions/node/v20.11.1",
                "20.11.1",
            ),
            (
                "fnm",
                ".local/share/fnm/node-versions/v24.22.1/installation",
                "24.22.1",
            ),
            ("n shared prefix", "opt/n-shared", "23.3.0"),
            ("Volta", ".volta/tools/image/node/22.12.0", "22.12.0"),
            ("asdf", ".asdf/installs/nodejs/22.13.0", "22.13.0"),
            ("mise", ".local/share/mise/installs/node/23.6.0", "23.6.0"),
            ("make install", "opt/custom-node", "25.0.0"),
        ] {
            let binary = install(&root.join(prefix), version);
            assert_eq!(
                version_of(&binary, user).as_deref(),
                Some(version),
                "{name}"
            );
        }
        let cellar = root.join("opt/homebrew/Cellar/node/26.13.0_2");
        install(&cellar, "26.13.0");
        std::fs::create_dir_all(root.join("opt/homebrew/bin")).unwrap();
        std::os::unix::fs::symlink(
            "../Cellar/node/26.13.0_2/bin/node",
            root.join("opt/homebrew/bin/node"),
        )
        .unwrap();
        assert_eq!(
            version_of(&root.join("opt/homebrew/bin/node"), user).as_deref(),
            Some("26.13.0")
        );
        std::fs::create_dir(root.join("links")).unwrap();
        std::os::unix::fs::symlink(
            root.join("node-v24.18.0-darwin-arm64/bin/node"),
            root.join("links/node"),
        )
        .unwrap();
        assert_eq!(
            version_of(&root.join("links/node"), user).as_deref(),
            Some("24.18.0")
        );
        for bundled in [
            "Tool.app/Contents/Resources/bin/node",
            "codegraph/versions/v1.6.2/node",
        ] {
            let binary = root.join(bundled);
            std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
            std::fs::write(&binary, "").unwrap();
            let tags = observed(&binary, user);
            assert!(tags.iter().any(|(d, _)| d == "runtime"), "{bundled}");
            assert!(tags.iter().all(|(d, _)| d != "version"), "{bundled}");
        }
        let sea = root.join("tool/sea-app");
        std::fs::create_dir_all(sea.parent().unwrap()).unwrap();
        std::fs::write(&sea, "").unwrap();
        assert!(observed(&sea, user).is_empty());
        temp.close().unwrap();
    }

    #[test]
    fn node_headers_are_read_once_per_prefix_and_need_all_numeric_parts() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let user = unsafe { libc::geteuid() };
        let binary = install(&root.join("good"), "24.18.0");
        let mut cache = Versions::new();
        assert_eq!(
            header_version(&binary, Some(user), &mut cache).as_deref(),
            Some("24.18.0")
        );
        std::fs::remove_file(root.join("good/include/node/node_version.h")).unwrap();
        assert_eq!(
            header_version(&binary, Some(user), &mut cache).as_deref(),
            Some("24.18.0")
        );
        assert_eq!(cache.len(), 1);
        let header = |name: &str| {
            let binary = install(&root.join(name), "1.2.3");
            (binary, root.join(name).join("include/node/node_version.h"))
        };
        let (malformed, file) = header("malformed");
        std::fs::write(
            &file,
            "#define NODE_MAJOR_VERSION 1\n#define NODE_MINOR_VERSION x\n#define NODE_PATCH_VERSION 3\n",
        )
        .unwrap();
        let (partial, file) = header("partial");
        std::fs::write(
            &file,
            "#define NODE_MAJOR_VERSION 1\n#define NODE_MINOR_VERSION 2\n",
        )
        .unwrap();
        let (owned, _) = header("owned");
        let mut rejected = vec![
            (malformed, Some(user)),
            (partial, Some(user)),
            (owned.clone(), None),
        ];
        if let Some(other) = files::tests::other_uid() {
            rejected.push((owned, Some(other)));
        }
        for (binary, owner) in rejected {
            assert_eq!(
                header_version(&binary, owner, &mut Versions::new()),
                None,
                "{binary:?} {owner:?}"
            );
        }
        temp.close().unwrap();
    }

    #[test]
    fn missing_node_header_keeps_the_unknown_version_presentation() {
        let user = unsafe { libc::geteuid() };
        let tree = crate::tree::build(vec![Process {
            pid: 81,
            ppid: Some(1),
            uid: Some(user),
            started_unix_us: Some(81),
            name: "node".into(),
            executable: Some("/nonexistent/prefix/bin/node".into()),
            ..Default::default()
        }])
        .unwrap();
        let node = crate::tree::select(&tree, "pid:81").unwrap();
        assert_eq!(node.display_name(), "node");
        let snapshot = crate::model::Snapshot {
            schema: crate::model::JSON_SCHEMA_VERSION,
            extensions: vec![],
            captured_unix_ms: 1,
            sample_ms: 1,
            collection_ms: 1,
            system: Default::default(),
            scope: "pid:81".into(),
            warnings: vec![],
            root: node,
        };
        assert!(crate::output::markdown(&snapshot).contains("> version ?"));
        let json = serde_json::to_value(&snapshot).unwrap();
        assert!(json["selections"][0]["root"]["process"]["version"].is_null());
    }

    #[test]
    fn entries_follow_deterministic_option_rules_and_existing_files() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        for file in ["a.js", "app.js", "x.js", "codegraph.js", "notes.txt"] {
            std::fs::write(root.join(file), "").unwrap();
        }
        let node = Path::new("/opt/node/bin/node");
        let codegraph = root.join("codegraph.js").to_string_lossy().into_owned();
        for (argv, expected) in [
            (vec!["node", "a.js"], Some("a.js")),
            (
                vec![
                    "node",
                    "--liftoff-only",
                    "--disable-warning=ExperimentalWarning",
                    &codegraph,
                    "serve",
                    "--mcp",
                ],
                Some("codegraph.js"),
            ),
            (
                vec![
                    "node",
                    "--max-old-space-size=4096",
                    "--inspect",
                    "a.js",
                    "--port",
                    "1",
                ],
                Some("a.js"),
            ),
            (vec!["node", "--no-warnings", "--", "a.js"], Some("a.js")),
            (
                vec!["node", "-C", "development", "--title", "srv", "a.js"],
                Some("a.js"),
            ),
            (vec!["node", "--unknown-flag", "a.js"], Some("a.js")),
            (vec!["node", "-x", "a.js"], Some("a.js")),
            (vec!["node", "."], Some("")),
            (vec!["node", "--unknown-flag", "value", "a.js"], None),
            (vec!["node", "--unknown-flag", "notes.txt"], None),
            (
                vec!["node", "--trace-event-categories", "v8", "app.js"],
                None,
            ),
            (
                vec!["node", "--experimental-default-type", "module", "x.js"],
                None,
            ),
            (vec!["node", "inspect", "x.js"], None),
            (vec!["node", "missing.js"], None),
            (vec!["node"], None),
            (vec!["node", "-"], None),
            (vec!["node", "-e", "code"], None),
            (vec!["node", "--eval=code"], None),
            (vec!["node", "-p", "1"], None),
            (vec!["node", "--print", "1"], None),
            (vec!["node", "-r", "hook", "a.js"], None),
            (vec!["node", "--require=hook", "a.js"], None),
            (vec!["node", "--import", "hook", "a.js"], None),
            (vec!["node", "--experimental-loader=hook", "a.js"], None),
            (vec!["node", "https://host/run.js"], None),
            (vec!["node", "token=secret"], None),
            (vec!["node", "{\"a\":1}"], None),
        ] {
            let found = entry(args(&argv), node, Some(&root));
            assert_eq!(
                found.map(|p| p
                    .strip_prefix(&root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()),
                expected.map(str::to_owned),
                "{argv:?}"
            );
        }
        assert!(entry(args(&["node", "a.js"]), node, None).is_none());
        temp.close().unwrap();
    }

    #[test]
    fn work_labels_use_package_names_or_specific_stems_only() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let installed = root.join("cache/node_modules/firecrawl-mcp/dist");
        let scoped = root.join("cache/node_modules/@scope/tool/bin");
        let bin = root.join("cache/node_modules/.bin");
        let project = root.join("project/src");
        let scripts = root.join("scripts");
        let elsewhere = root.join("elsewhere");
        for dir in [&installed, &scoped, &bin, &project, &scripts, &elsewhere] {
            std::fs::create_dir_all(dir).unwrap();
        }
        for file in [
            installed.join("index.js"),
            scoped.join("run.js"),
            project.join("index.js"),
            project.join("other.js"),
            scripts.join("sync-mail.js"),
            scripts.join("index.js"),
            scripts.join("deploy-token.js"),
            scripts.join("a3f9c2e1b4d5a6f7e8c9.js"),
            scripts.join("job-QmFzZTY0U2VjcmV0VmFsdWU1.js"),
            elsewhere.join("sync-mail.js"),
        ] {
            std::fs::write(file, "").unwrap();
        }
        std::os::unix::fs::symlink(installed.join("index.js"), bin.join("firecrawl-mcp")).unwrap();
        std::fs::write(
            root.join("project/package.json"),
            r#"{"name":"chrome-devtools-mcp"}"#,
        )
        .unwrap();
        let node = Path::new("/opt/node/bin/node");
        let user = unsafe { libc::geteuid() };
        let mut cache = Packages::new();
        let mut label = |value: &Path, cwd: Option<&Path>| {
            entry(args(&["node", &value.to_string_lossy()]), node, cwd)
                .and_then(|path| work(&path, user, &mut cache))
        };
        let package = label(&installed.join("index.js"), None).unwrap();
        assert_eq!(
            (package.1.as_str(), package.2),
            ("firecrawl-mcp", "argv_package")
        );
        assert_eq!(label(&bin.join("firecrawl-mcp"), None).unwrap(), package);
        assert_eq!(
            label(&scoped.join("run.js"), None).unwrap().1,
            "@scope/tool"
        );
        let json = label(Path::new("src/index.js"), Some(&root.join("project"))).unwrap();
        assert_eq!(
            (json.1.as_str(), json.2),
            ("chrome-devtools-mcp", "argv_package_json")
        );
        let directory = label(Path::new("."), Some(&root.join("project"))).unwrap();
        assert_eq!(directory.1, "chrome-devtools-mcp");
        let other = label(&project.join("other.js"), None).unwrap();
        assert_eq!(other.1, "chrome-devtools-mcp");
        assert_ne!(other.0, json.0);
        let script = label(&scripts.join("sync-mail.js"), None).unwrap();
        assert_eq!((script.1.as_str(), script.2), ("sync-mail", "argv_script"));
        assert_ne!(
            label(&elsewhere.join("sync-mail.js"), None).unwrap().0,
            script.0
        );
        for rejected in [
            "index.js",
            "deploy-token.js",
            "a3f9c2e1b4d5a6f7e8c9.js",
            "job-QmFzZTY0U2VjcmV0VmFsdWU1.js",
        ] {
            assert!(label(&scripts.join(rejected), None).is_none(), "{rejected}");
        }
        temp.close().unwrap();
    }

    #[test]
    fn nearest_package_json_owns_the_entry_in_any_order_and_must_be_safe() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let dist = root.join("pkg/dist");
        std::fs::create_dir_all(&dist).unwrap();
        std::fs::write(root.join("pkg/package.json"), r#"{"name":"named-pkg"}"#).unwrap();
        std::fs::write(dist.join("package.json"), r#"{"type":"module"}"#).unwrap();
        std::fs::write(dist.join("alpha.js"), "").unwrap();
        std::fs::write(root.join("pkg/beta.js"), "").unwrap();
        let user = unsafe { libc::geteuid() };
        let label = |order: [&Path; 2]| {
            let mut cache = Packages::new();
            let mut labels: Vec<_> = order
                .iter()
                .map(|path| work(path, user, &mut cache).map(|w| w.1))
                .collect();
            labels.sort();
            labels
        };
        let alpha = dist.join("alpha.js");
        let beta = root.join("pkg/beta.js");
        let expected = vec![Some("alpha".to_owned()), Some("named-pkg".to_owned())];
        assert_eq!(label([&alpha, &beta]), expected);
        assert_eq!(label([&beta, &alpha]), expected);
        let piped = root.join("piped");
        std::fs::create_dir(&piped).unwrap();
        let fifo = std::ffi::CString::new(piped.join("package.json").to_str().unwrap()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o600) }, 0);
        std::fs::write(piped.join("gamma.js"), "").unwrap();
        let (sender, receiver) = std::sync::mpsc::channel();
        let gamma = piped.join("gamma.js");
        std::thread::spawn(move || {
            let mut cache = Packages::new();
            sender
                .send(work(&gamma, user, &mut cache).map(|w| w.1))
                .unwrap();
        });
        assert_eq!(
            receiver
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap(),
            Some("gamma".to_owned())
        );
        if let Some(other) = files::tests::other_uid() {
            let mut cache = Packages::new();
            assert_eq!(
                work(&beta, other, &mut cache).map(|w| w.1),
                Some("beta".to_owned())
            );
            assert_eq!(cache[&(root.join("pkg"), other)], Package::Unnamed);
        }
        std::fs::remove_file(piped.join("package.json")).unwrap();
        temp.close().unwrap();
    }

    struct Counting(usize, Option<Vec<String>>);
    impl Argv for Counting {
        fn read(&mut self, _: &Process) -> Option<Vec<String>> {
            self.0 += 1;
            self.1.clone()
        }
    }

    #[test]
    fn argv_is_read_once_only_for_own_identified_node_processes() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        std::fs::write(root.join("sync-mail.js"), "").unwrap();
        let user = unsafe { libc::geteuid() };
        let fnm = install(
            &root.join(".local/share/fnm/node-versions/v24.18.0/installation"),
            "24.18.0",
        );
        let node = |pid: u32| Process {
            pid,
            ppid: Some(1),
            uid: Some(user),
            started_unix_us: Some(u64::from(pid)),
            name: "node".into(),
            executable: Some(fnm.clone()),
            cwd: Some(root.clone()),
            ..Default::default()
        };
        let mut other_user = node(11);
        other_user.uid = Some(user + 1);
        let mut unverified = node(12);
        unverified.started_unix_us = None;
        let mut not_node = node(13);
        not_node.executable = Some("/opt/tool/bin/python3".into());
        let mut detailed = node(14);
        detailed.command = Some(crate::model::ProcessCommand {
            argv: Some(args(&["node", "sync-mail.js"])),
            ..Default::default()
        });
        let processes = vec![node(10), other_user, unverified, not_node, detailed];
        let facts = Facts::new(&processes);
        let secret = args(&[
            "node",
            "--max-old-space-size=4096",
            "sync-mail.js",
            "--token",
            "hunter2",
        ]);
        let mut reader = Counting(0, Some(secret));
        let proposals = proposals(&facts, true, &mut reader);
        assert_eq!(reader.0, 1);
        let text = serde_json::to_string(&proposals).unwrap();
        for leaked in [
            "hunter2",
            "--token",
            "max-old-space",
            "sync-mail.js",
            root.to_str().unwrap(),
        ] {
            assert!(!text.contains(leaked), "{leaked}: {text}");
        }
        let tags: Vec<_> = proposals
            .iter()
            .filter_map(|p| match &p.evidence {
                crate::extensions::ClassificationEvidence::Tags { proposal } => {
                    match &proposal.operation {
                        crate::extensions::tags::Operation::Add { tag } => {
                            Some((p.identity.pid, tag.clone()))
                        }
                        _ => None,
                    }
                }
                _ => None,
            })
            .collect();
        let of = |pid: u32, dimension: &str| {
            tags.iter()
                .find(|(p, t)| *p == pid && t.dimension == dimension)
                .map(|(_, t)| t.clone())
        };
        for pid in [10, 14] {
            let work = of(pid, "work").unwrap();
            assert_eq!(
                (work.label.as_str(), work.basis.as_str(), work.path.as_ref()),
                ("sync-mail", "argv_script", None)
            );
            assert_eq!(of(pid, "runtime").unwrap().id, "runtime:node");
            assert_eq!(
                of(pid, "version").unwrap().value.as_deref(),
                Some("24.18.0")
            );
        }
        assert_eq!(of(10, "work").unwrap().id, of(14, "work").unwrap().id);
        assert!(of(11, "work").is_none() && of(11, "runtime").is_some());
        assert!(tags.iter().all(|(pid, _)| *pid != 12 && *pid != 13));
        let mut reader = Counting(0, Some(args(&["node", "sync-mail.js"])));
        let off = super::proposals(&facts, false, &mut reader);
        assert_eq!(reader.0, 0);
        assert!(
            off.iter()
                .all(|p| !serde_json::to_string(p).unwrap().contains("\"work\""))
        );
        let mut changed = Counting(0, None);
        let lost = super::proposals(&facts, true, &mut changed);
        assert_eq!(changed.0, 1);
        assert_eq!(
            lost.iter()
                .filter(|p| serde_json::to_string(p).unwrap().contains("\"work\""))
                .count(),
            1
        );
        temp.close().unwrap();
    }

    #[test]
    fn builtin_setting_controls_argv_candidates_reported_per_round() {
        let user = unsafe { libc::geteuid() };
        let process = Process {
            pid: 42,
            ppid: Some(1),
            uid: Some(user),
            started_unix_us: Some(42),
            name: "node".into(),
            executable: Some("/opt/node/bin/node".into()),
            cwd: Some("/".into()),
            ..Default::default()
        };
        let registry = crate::extensions::settings_registry().unwrap();
        let cancel = crate::snapshot::Cancellation::default();
        for (value, expected) in [(true, Some(1)), (false, None)] {
            let overrides = registry
                .parse_overrides(&[format!("{}={value}", entry_setting("node"))])
                .unwrap();
            let settings = registry
                .resolve(&crate::settings::Config::default(), &overrides)
                .unwrap();
            let context = crate::extensions::Context::new(&settings, &cancel);
            crate::tree::build_with_context(vec![process.clone()], &context).unwrap();
            let records = context.report.lock().unwrap().clone();
            let record = records
                .iter()
                .find(|r| r.provider == "node" && r.phase == "classify")
                .unwrap();
            assert_eq!(
                record.argv.as_ref().map(|a| a.candidates),
                expected,
                "{value}"
            );
            assert!(record.argv.as_ref().is_none_or(|a| a.reads == 0));
            let profile = crate::output::extension_profile(&records);
            assert_eq!(profile.contains("## Argument reads"), value, "{profile}");
            let json = serde_json::to_value(&records).unwrap();
            let schema: serde_json::Value =
                serde_json::from_str(crate::output::JSON_SCHEMA).unwrap();
            let mut extension_schema = schema["properties"]["extensions"].clone();
            extension_schema["$defs"] = schema["$defs"].clone();
            let validator = jsonschema::validator_for(&extension_schema).unwrap();
            assert!(validator.is_valid(&json), "{json}");
        }
    }

    #[test]
    fn entry_reading_is_a_separate_setting_that_defaults_on() {
        let registry = crate::extensions::settings_registry().unwrap();
        let on = registry
            .resolve(
                &crate::settings::Config::default(),
                &std::collections::BTreeMap::new(),
            )
            .unwrap();
        assert!(entries_enabled(&on, "node"));
        let off = registry
            .resolve(
                &crate::settings::Config {
                    settings: [(entry_setting("node"), serde_json::json!(false))].into(),
                    ..Default::default()
                },
                &std::collections::BTreeMap::new(),
            )
            .unwrap();
        assert!(!entries_enabled(&off, "node"));
        let overrides = registry
            .parse_overrides(&[format!("{}=false", entry_setting("node"))])
            .unwrap();
        assert!(!entries_enabled(
            &registry
                .resolve(&crate::settings::Config::default(), &overrides)
                .unwrap(),
            "node"
        ));
    }

    pub(crate) fn work_tree() -> (tempfile::TempDir, crate::model::Node) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let project = root.join("project");
        std::fs::create_dir_all(project.join(".git")).unwrap();
        let user = unsafe { libc::geteuid() };
        let homebrew = install(&root.join("opt/homebrew/Cellar/node/22.1.0"), "22.1.0");
        let entry = |dir: &str, package: &str| {
            let file = root
                .join(dir)
                .join("node_modules")
                .join(package)
                .join("dist/index.js");
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(&file, "").unwrap();
            file.to_string_lossy().into_owned()
        };
        let node = |pid: u32, ppid: u32, entry: Option<String>| Process {
            pid,
            ppid: Some(ppid),
            uid: Some(user),
            started_unix_us: Some(u64::from(pid)),
            name: "node".into(),
            executable: Some(homebrew.clone()),
            cwd: Some(root.clone()),
            resident_bytes: Some(100),
            command: Some(crate::model::ProcessCommand {
                argv: Some(match entry {
                    Some(entry) => args(&["node", &entry]),
                    None => args(&["node", "-e", "1"]),
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        let mut in_project = node(17, 1, Some(entry("a", "firecrawl-mcp")));
        in_project.cwd = Some(project.clone());
        let processes = vec![
            node(10, 1, Some(entry("a", "codegraph"))),
            node(11, 1, Some(entry("a", "firecrawl-mcp"))),
            node(12, 1, Some(entry("a", "chrome-devtools-mcp"))),
            node(13, 1, Some(entry("a", "firecrawl-mcp"))),
            node(14, 1, Some(entry("b", "firecrawl-mcp"))),
            node(15, 11, Some(entry("a", "codegraph"))),
            node(16, 11, None),
            in_project,
        ];
        (temp, crate::tree::build(processes).unwrap())
    }

    #[test]
    fn work_groups_split_shared_node_runtimes_without_duplicating_processes() {
        let (temp, tree) = work_tree();
        assert_eq!(tree.totals.processes, 8);
        assert_eq!(tree.totals.resident_bytes, 800);
        let mut seen = std::collections::BTreeMap::new();
        let mut pending = vec![&tree];
        let mut works = Vec::new();
        while let Some(node) = pending.pop() {
            if let Some(process) = &node.process {
                *seen.entry(process.pid).or_insert(0) += 1;
            }
            if matches!(node.grouping, Some(crate::model::Grouping::Work)) {
                works.push((node.name.clone(), node.totals.processes));
            }
            pending.extend(&node.children);
        }
        assert_eq!(seen.len(), 8);
        assert!(seen.values().all(|count| *count == 1));
        works.sort();
        assert_eq!(
            works,
            [("codegraph".to_owned(), 2), ("firecrawl-mcp".to_owned(), 2)]
        );
        let unknown = crate::tree::select(&tree, "group:unclassified").unwrap();
        let names: Vec<_> = unknown.children.iter().map(|n| n.display_name()).collect();
        for expected in ["chrome-devtools-mcp", "firecrawl-mcp", "node"] {
            assert!(names.iter().any(|n| n == expected), "{expected}: {names:?}");
        }
        let firecrawl = unknown
            .children
            .iter()
            .find(|n| n.name == "firecrawl-mcp" && n.totals.processes == 2)
            .unwrap();
        assert!(crate::tree::select(firecrawl, "pid:16").is_err());
        assert!(crate::tree::select(firecrawl, "pid:15").is_err());
        let child = crate::tree::select(&tree, "pid:16").unwrap();
        assert_eq!(child.process.as_ref().unwrap().ppid, Some(11));
        assert_eq!(child.display_name(), "node");
        let projects = crate::tree::select(&tree, "group:projects").unwrap();
        let row = crate::tree::select(&projects, "pid:17").unwrap();
        assert_eq!(row.display_name(), "firecrawl-mcp");
        let process = row.process.unwrap();
        let tags = crate::output::effective_tags_text(process.tag_report.as_ref());
        assert!(tags.contains("#node"), "{tags}");
        assert!(tags.contains("work firecrawl-mcp (argv_package)"), "{tags}");
        assert!(tags.contains("version node 22.1.0"), "{tags}");
        assert_eq!(process.original_name.as_deref(), Some("node"));
        assert_eq!(process.version.as_ref().unwrap().value, "22.1.0");
        assert_eq!(
            process.name_source.unwrap().reason,
            crate::model::NameReason::Work
        );
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
        assert!(
            markdown.contains("- firecrawl-mcp [work] group:"),
            "{markdown}"
        );
        assert!(!markdown.contains("firecrawl-mcp 22.1.0"), "{markdown}");
        assert!(!markdown.contains("dist/index.js"), "{markdown}");
        let schema: serde_json::Value = serde_json::from_str(crate::output::JSON_SCHEMA).unwrap();
        let validator = jsonschema::validator_for(&schema).unwrap();
        for pid in [10, 11, 12, 13, 14, 15, 16, 17] {
            let selected = crate::model::Snapshot {
                schema: crate::model::JSON_SCHEMA_VERSION,
                extensions: vec![],
                captured_unix_ms: 1,
                sample_ms: 1,
                collection_ms: 1,
                system: Default::default(),
                scope: format!("pid:{pid}"),
                warnings: vec![],
                root: crate::tree::select(&tree, &format!("pid:{pid}")).unwrap(),
            };
            let value = serde_json::to_value(selected).unwrap();
            let errors: Vec<_> = validator
                .iter_errors(&value)
                .map(|e| e.to_string())
                .collect();
            assert!(errors.is_empty(), "pid {pid}: {errors:?}");
        }
        let json = serde_json::to_value(crate::model::Snapshot {
            schema: crate::model::JSON_SCHEMA_VERSION,
            extensions: vec![],
            captured_unix_ms: 1,
            sample_ms: 1,
            collection_ms: 1,
            system: Default::default(),
            scope: "group:all".into(),
            warnings: vec![],
            root: tree,
        })
        .unwrap();
        let schema: serde_json::Value = serde_json::from_str(crate::output::JSON_SCHEMA).unwrap();
        let errors: Vec<_> = jsonschema::validator_for(&schema)
            .unwrap()
            .iter_errors(&json)
            .map(|e| e.to_string().chars().take(200).collect::<String>())
            .collect();
        assert!(errors.is_empty(), "{errors:?}");
        temp.close().unwrap();
    }
}
