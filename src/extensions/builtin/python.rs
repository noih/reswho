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

pub(crate) const ID: &str = "python";

pub(crate) static MODULE: Module = Module {
    summary: None,
    registration,
    decorate: None,
    id: ID,
    capabilities: &[Capability::Classify],
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

const GENERIC_STEMS: &[&str] = &["main", "__main__", "app", "run", "server", "cli", "start"];
const FLAGS: &[char] = &[
    'b', 'B', 'd', 'E', 'i', 'I', 'O', 'P', 'q', 'R', 's', 'S', 'u', 'v', 'x',
];
const VALUE_SHORT: &[char] = &['W', 'X'];
const VALUE_LONG: &[&str] = &["--check-hash-based-pycs"];

fn abi(name: &str) -> Option<(&str, &str)> {
    let suffix = name.strip_prefix("python")?;
    let minor = suffix
        .strip_suffix('t')
        .filter(|minor| minor.contains('.'))
        .unwrap_or(suffix);
    minor
        .chars()
        .all(|c| c.is_ascii_digit() || c == '.')
        .then_some((minor, suffix))
}

pub(crate) fn python_executable(executable: &Path) -> bool {
    let Some(name) = executable.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let framework = name == "Python"
        && executable
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .and_then(|bundle| bundle.file_name())
            .is_some_and(|bundle| bundle == "Python.app");
    abi(name).is_some() || framework
}

pub(crate) fn prefix(executable: &Path) -> Option<(PathBuf, String, String)> {
    let parts: Vec<_> = executable.components().collect();
    let name = executable.file_name()?.to_str()?;
    let framework = parts.windows(3).position(|window| {
        matches!(window, [Component::Normal(framework), Component::Normal(versions), _]
            if (*framework == "Python.framework"
                || *framework == "Python3.framework"
                || *framework == "PythonT.framework")
                && *versions == "Versions")
    });
    if let Some(at) = framework {
        let minor = parts[at + 2].as_os_str().to_str()?.to_owned();
        let prefix: PathBuf = parts[..at + 3].iter().collect();
        let include = if name == format!("python{minor}t") {
            format!("python{minor}t")
        } else {
            format!("python{minor}")
        };
        return Some((prefix, minor, include));
    }
    let bin = executable.parent()?;
    let (minor, suffix) = abi(name).filter(|(minor, _)| minor.contains('.'))?;
    (bin.file_name()? == "bin")
        .then(|| bin.parent().map(Path::to_path_buf))
        .flatten()
        .map(|prefix| (prefix, minor.to_owned(), format!("python{suffix}")))
}

pub(crate) type Versions = HashMap<(PathBuf, Option<u32>), Option<String>>;

pub(crate) fn header_version(
    executable: &Path,
    owner: Option<u32>,
    cache: &mut Versions,
) -> Option<String> {
    let (prefix, minor, include) = prefix(executable)?;
    let header = prefix.join(format!("include/{include}/patchlevel.h"));
    cache
        .entry((header.clone(), owner))
        .or_insert_with(|| {
            let files::Trusted::Bytes(bytes) = files::read(&header, owner) else {
                return None;
            };
            let text = String::from_utf8(bytes).ok()?;
            let value = files::define(&text, "PY_VERSION")?
                .strip_prefix('"')?
                .strip_suffix('"')?;
            (files::version_text(value) && value.starts_with(&format!("{minor}.")))
                .then(|| value.to_owned())
        })
        .clone()
}

pub(crate) enum Entry {
    Script(std::path::PathBuf),
    Module(String),
}

fn module(name: &str) -> bool {
    !name.is_empty()
        && name.split('.').all(|part| {
            part.chars()
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                && part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
}

pub(crate) fn entry(argv: Vec<String>, cwd: Option<&Path>) -> Option<Entry> {
    let command = sanitize_command(argv.clone(), Some(Path::new("python3")));
    let argv = &argv;
    let usable = |index: usize| !command.redacted_indices.contains(&index);
    let mut index = 1;
    while let Some(arg) = argv.get(index) {
        if !arg.starts_with('-') {
            break;
        }
        if VALUE_LONG.contains(&arg.as_str()) {
            index += 2;
            continue;
        }
        if arg.starts_with("--") || arg == "-" {
            return None;
        }
        let mut chars = arg[1..].chars();
        loop {
            match chars.next() {
                None => {
                    index += 1;
                    break;
                }
                Some(c) if FLAGS.contains(&c) => continue,
                Some(c) if VALUE_SHORT.contains(&c) => {
                    index += if chars.as_str().is_empty() { 2 } else { 1 };
                    break;
                }
                Some('m') => {
                    let rest = chars.as_str();
                    let (name, at) = if rest.is_empty() {
                        (argv.get(index + 1)?.as_str(), index + 1)
                    } else {
                        (rest, index)
                    };
                    return (usable(at) && module(name) && !suspicious(name))
                        .then(|| Entry::Module(name.to_owned()));
                }
                Some(_) => return None,
            }
        }
    }
    let value = argv.get(index)?;
    if value.is_empty() || !usable(index) {
        return None;
    }
    resolve(value, cwd)
        .filter(|path| path.is_file())
        .map(Entry::Script)
}

pub(crate) fn work(entry: &Entry) -> Option<(String, String, &'static str)> {
    let (key, label, kind) = match entry {
        Entry::Module(name) => (format!("module:{name}"), name.clone(), "argv_module"),
        Entry::Script(path) => {
            let stem = path.file_stem()?.to_str()?;
            if GENERIC_STEMS.contains(&stem.to_ascii_lowercase().as_str()) || suspicious(stem) {
                return None;
            }
            (
                path.to_string_lossy().into_owned(),
                stem.to_owned(),
                "argv_script",
            )
        }
    };
    let id = format!(
        "work:{}",
        stable_ref("work", &key).trim_start_matches("group:")
    );
    Some((id, label, kind))
}

pub(crate) fn proposals(
    facts: &Facts<'_>,
    entries: bool,
    argv: &mut dyn Argv,
) -> Vec<ClassificationProposal> {
    let user = unsafe { libc::geteuid() };
    let mut proposals = Vec::new();
    let mut versions = Versions::new();
    for process in facts.processes {
        let Some(identity) = ProcessIdentity::observed(process) else {
            continue;
        };
        let Some(executable) = facts.executables.get(&process.pid) else {
            continue;
        };
        if !python_executable(executable) {
            continue;
        }
        proposals.push(with_id(
            identity_proposal(identity, ID, "executable", "runtime", ID, None),
            "runtime:python".into(),
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
            .and_then(|args| entry(args, process.cwd.as_deref()))
            .and_then(|entry| work(&entry))
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
mod tests {
    use super::*;
    use crate::model::Process;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| (*v).to_owned()).collect()
    }

    const FRAMEWORK: &str = "/opt/homebrew/Cellar/python@3.14/3.14.10/Frameworks/Python.framework/Versions/3.14/Resources/Python.app/Contents/MacOS/Python";

    fn patchlevel(prefix: &Path, minor: &str, version: &str) {
        let include = prefix.join(format!("include/python{minor}"));
        std::fs::create_dir_all(&include).unwrap();
        std::fs::write(
            include.join("patchlevel.h"),
            format!("#define PY_MAJOR_VERSION 3\n#define PY_VERSION \"{version}\"\n#define PY_SECRET \"x\"\n"),
        )
        .unwrap();
    }

    #[test]
    fn python_versions_come_from_the_interpreter_patchlevel_header() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let user = unsafe { libc::geteuid() };
        let layouts = [
            (
                "python.org",
                "Library/Frameworks/Python.framework/Versions/3.13",
                "bin/python3.13",
                "3.13",
                "3.13.2",
            ),
            (
                "Homebrew",
                "Cellar/python@3.14/3.14.10/Frameworks/Python.framework/Versions/3.14",
                "Resources/Python.app/Contents/MacOS/Python",
                "3.14",
                "3.14.10",
            ),
            (
                "uv",
                "uv/python/cpython-3.11.16-macos-aarch64-none",
                "bin/python3.11",
                "3.11",
                "3.11.16",
            ),
            (
                "Apple CLT",
                "CommandLineTools/Library/Frameworks/Python3.framework/Versions/3.9",
                "bin/python3.9",
                "3.9",
                "3.9.6",
            ),
            (
                "pyenv",
                ".pyenv/versions/3.12.4",
                "bin/python3.12",
                "3.12",
                "3.12.4",
            ),
            (
                "asdf",
                ".asdf/installs/python/3.12.5",
                "bin/python3.12",
                "3.12",
                "3.12.5",
            ),
            (
                "mise",
                ".local/share/mise/installs/python/3.12.6",
                "bin/python3.12",
                "3.12",
                "3.12.6",
            ),
            (
                "conda",
                "miniforge3/envs/work",
                "bin/python3.11",
                "3.11",
                "3.11.9",
            ),
            (
                "uv free-threaded",
                "uv/python/cpython-3.13.2+freethreaded-macos-aarch64-none",
                "bin/python3.13t",
                "3.13t",
                "3.13.2",
            ),
            (
                "python.org free-threaded",
                "Library/Frameworks/PythonT.framework/Versions/3.13",
                "bin/python3.13t",
                "3.13t",
                "3.13.2",
            ),
        ];
        let mut cache = Versions::new();
        for (name, prefix, binary, minor, version) in layouts {
            let prefix = root.join(prefix);
            patchlevel(&prefix, minor, version);
            let executable = prefix.join(binary);
            std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
            std::fs::write(&executable, "").unwrap();
            assert_eq!(
                header_version(&executable, Some(user), &mut cache).as_deref(),
                Some(version),
                "{name}"
            );
        }
        let base = root.join("venv-base");
        patchlevel(&base, "3.12", "3.12.7");
        std::fs::create_dir_all(base.join("bin")).unwrap();
        std::fs::write(base.join("bin/python3.12"), "").unwrap();
        let venv = root.join("project/.venv");
        std::fs::create_dir_all(venv.join("bin")).unwrap();
        std::fs::write(
            venv.join("pyvenv.cfg"),
            format!("home = {}\n", base.join("bin").display()),
        )
        .unwrap();
        std::os::unix::fs::symlink(base.join("bin/python3.12"), venv.join("bin/python3.12"))
            .unwrap();
        std::os::unix::fs::symlink("python3.12", venv.join("bin/python")).unwrap();
        let resolved = venv.join("bin/python").canonicalize().unwrap();
        assert_eq!(
            header_version(&resolved, Some(user), &mut Versions::new()).as_deref(),
            Some("3.12.7")
        );
        let bundled = root.join("Tool.app/Contents/Resources/python/bin/python3.12");
        std::fs::create_dir_all(bundled.parent().unwrap()).unwrap();
        let mismatched = root.join("mismatched");
        patchlevel(&mismatched, "3.12", "3.13.0");
        let default_build = root.join("default-build");
        patchlevel(&default_build, "3.13", "3.13.2");
        let uv = root.join("uv/python/cpython-3.11.16-macos-aarch64-none");
        let mut rejected = vec![
            (bundled, Some(user)),
            (uv.join("bin/python3"), Some(user)),
            (mismatched.join("bin/python3.12"), Some(user)),
            (default_build.join("bin/python3.13t"), Some(user)),
            (uv.join("bin/python3.11"), None),
        ];
        if let Some(other) = files::tests::other_uid() {
            rejected.push((uv.join("bin/python3.11"), Some(other)));
        }
        for (path, owner) in rejected {
            assert_eq!(
                header_version(&path, owner, &mut cache),
                None,
                "{path:?} {owner:?}"
            );
        }
        let reads = cache.len();
        header_version(&uv.join("bin/python3.11"), Some(user), &mut cache);
        assert_eq!(cache.len(), reads);
        temp.close().unwrap();
    }

    #[test]
    fn missing_python_header_keeps_the_unknown_version_presentation() {
        let user = unsafe { libc::geteuid() };
        let tree = crate::tree::build(vec![Process {
            pid: 80,
            ppid: Some(1),
            uid: Some(user),
            started_unix_us: Some(80),
            name: "python3.12".into(),
            executable: Some("/nonexistent/prefix/bin/python3.12".into()),
            ..Default::default()
        }])
        .unwrap();
        let node = crate::tree::select(&tree, "pid:80").unwrap();
        assert_eq!(node.display_name(), "python3.12");
        let snapshot = crate::model::Snapshot {
            schema: crate::model::JSON_SCHEMA_VERSION,
            extensions: vec![],
            captured_unix_ms: 1,
            sample_ms: 1,
            collection_ms: 1,
            system: Default::default(),
            scope: "pid:80".into(),
            warnings: vec![],
            root: node,
        };
        assert!(crate::output::markdown(&snapshot).contains("> version ?"));
        let json = serde_json::to_value(&snapshot).unwrap();
        assert!(json["selections"][0]["root"]["process"]["version"].is_null());
    }

    #[test]
    fn python_executables_include_versioned_names_and_framework_apps() {
        for (path, expected) in [
            ("/usr/bin/python3.12", true),
            ("/Users/u/work/.venv/bin/python", true),
            (
                "/x/Python.framework/Versions/3.14/Resources/Python.app/Contents/MacOS/Python",
                true,
            ),
            ("/opt/uv/bin/python3.13t", true),
            ("/opt/tool/bin/pythont", false),
            ("/opt/tool/bin/python3t", false),
            ("/opt/tool/bin/pythonista", false),
            ("/Applications/Tool.app/Contents/MacOS/Python", false),
        ] {
            assert_eq!(python_executable(Path::new(path)), expected, "{path}");
        }
    }

    #[test]
    fn entries_accept_scripts_and_modules_with_known_options_only() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        for file in [
            "sync_mail.py",
            "main.py",
            "__main__.py",
            "deploy_token.py",
            "worker",
        ] {
            std::fs::write(root.join(file), "").unwrap();
        }
        std::fs::create_dir(root.join("pkgdir")).unwrap();
        let label = |argv: &[&str]| {
            entry(args(argv), Some(&root))
                .and_then(|entry| work(&entry))
                .map(|(_, label, kind)| (label, kind))
        };
        for (argv, expected) in [
            (
                vec!["python3", "sync_mail.py"],
                Some(("sync_mail", "argv_script")),
            ),
            (
                vec![
                    "python3",
                    "-u",
                    "-B",
                    "-X",
                    "dev",
                    "-Wignore",
                    "sync_mail.py",
                    "--token",
                    "x",
                ],
                Some(("sync_mail", "argv_script")),
            ),
            (
                vec!["python3", "-uBs", "-W", "error", "worker"],
                Some(("worker", "argv_script")),
            ),
            (
                vec![
                    "python3",
                    "--check-hash-based-pycs",
                    "never",
                    "sync_mail.py",
                ],
                Some(("sync_mail", "argv_script")),
            ),
            (
                vec!["python3", "-m", "http.server", "8000"],
                Some(("http.server", "argv_module")),
            ),
            (
                vec!["python3", "-I", "-mmcp_server_fetch"],
                Some(("mcp_server_fetch", "argv_module")),
            ),
            (vec!["python3", "-X", "dev", "-u", "sync_mail.py"], None),
            (vec!["python3", "-X", "dev", "-u", "-mworker_pool"], None),
            (vec!["python3", "main.py"], None),
            (vec!["python3", "__main__.py"], None),
            (vec!["python3", "deploy_token.py"], None),
            (vec!["python3", "missing.py"], None),
            (vec!["python3", "pkgdir"], None),
            (vec!["python3"], None),
            (vec!["python3", "-"], None),
            (vec!["python3", "-c", "print(1)"], None),
            (vec!["python3", "-uc", "print(1)"], None),
            (vec!["python3", "--unknown", "sync_mail.py"], None),
            (vec!["python3", "-Z", "sync_mail.py"], None),
            (vec!["python3", "-m", "token=secret"], None),
            (vec!["python3", "-m", "not-a-module"], None),
            (vec!["python3", "https://host/x.py"], None),
        ] {
            assert_eq!(
                label(&argv).map(|(l, k)| (l, k.to_owned())),
                expected.map(|(l, k)| (l.to_owned(), k.to_owned())),
                "{argv:?}"
            );
        }
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
    fn argv_is_read_once_only_for_own_identified_python_processes() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        std::fs::write(root.join("sync_mail.py"), "").unwrap();
        let interpreter = root.join("pyenv/versions/3.12.4");
        patchlevel(&interpreter, "3.12", "3.12.4");
        let executable = interpreter.join("bin/python3.12");
        let user = unsafe { libc::geteuid() };
        let python = |pid: u32| Process {
            pid,
            ppid: Some(1),
            uid: Some(user),
            started_unix_us: Some(u64::from(pid)),
            name: "python3.12".into(),
            executable: Some(executable.clone()),
            cwd: Some(root.clone()),
            ..Default::default()
        };
        let mut other_user = python(11);
        other_user.uid = Some(user + 1);
        let mut unverified = python(12);
        unverified.started_unix_us = None;
        let mut detailed = python(13);
        detailed.command = Some(crate::model::ProcessCommand {
            argv: Some(args(&["python3", "-m", "worker_pool"])),
            ..Default::default()
        });
        let processes = vec![python(10), other_user, unverified, detailed];
        let facts = Facts::new(&processes);
        let secret = args(&["python3", "sync_mail.py", "--password", "hunter2"]);
        let mut reader = Counting(0, Some(secret.clone()));
        let found = proposals(&facts, true, &mut reader);
        assert_eq!(reader.0, 1);
        let text = serde_json::to_string(&found).unwrap();
        for leaked in [
            "hunter2",
            "--password",
            "sync_mail.py",
            root.to_str().unwrap(),
        ] {
            assert!(!text.contains(leaked), "{leaked}");
        }
        let works: Vec<_> = found
            .iter()
            .filter_map(|p| match &p.evidence {
                crate::extensions::ClassificationEvidence::Tags { proposal } => {
                    match &proposal.operation {
                        crate::extensions::tags::Operation::Add { tag }
                            if tag.dimension == "work" =>
                        {
                            Some((p.identity.pid, tag.label.clone(), tag.basis.clone()))
                        }
                        _ => None,
                    }
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            works,
            [
                (10, "sync_mail".to_owned(), "argv_script".to_owned()),
                (13, "worker_pool".to_owned(), "argv_module".to_owned())
            ]
        );
        assert!(text.contains("runtime:python") && text.contains("3.12.4"));
        let mut off = Counting(0, Some(secret));
        proposals(&facts, false, &mut off);
        assert_eq!(off.0, 0);
        let mut reused = Counting(0, None);
        assert!(
            proposals(&facts, true, &mut reused)
                .iter()
                .filter(|p| serde_json::to_string(p).unwrap().contains("\"work\""))
                .count()
                == 1
        );
        assert_eq!(reused.0, 1);
        temp.close().unwrap();
    }

    #[test]
    fn python_app_bundle_version_wins_and_python_version_is_superseded() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let executable = root.join(FRAMEWORK.trim_start_matches("/opt/homebrew/"));
        std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
        std::fs::write(&executable, "").unwrap();
        patchlevel(executable.ancestors().nth(5).unwrap(), "3.14", "3.14.10");
        let mut info = plist::Dictionary::new();
        for (key, value) in [
            ("CFBundleIdentifier", "org.python.python"),
            ("CFBundleShortVersionString", "3.14.10"),
            ("CFBundleExecutable", "Python"),
        ] {
            info.insert(key.into(), value.into());
        }
        plist::Value::Dictionary(info)
            .to_file_xml(executable.ancestors().nth(2).unwrap().join("Info.plist"))
            .unwrap();
        let tree = crate::tree::build(vec![Process {
            pid: 20,
            ppid: Some(1),
            uid: Some(unsafe { libc::geteuid() }),
            started_unix_us: Some(20),
            name: "Python".into(),
            executable: Some(executable),
            resident_bytes: Some(1),
            ..Default::default()
        }])
        .unwrap();
        let process = crate::tree::select(&tree, "pid:20")
            .unwrap()
            .process
            .unwrap();
        let version = process.version.unwrap();
        assert_eq!(
            (
                version.subject.as_str(),
                version.value.as_str(),
                version.source.as_str()
            ),
            ("Python", "3.14.10", "app_bundle")
        );
        let report = process.tag_report.unwrap();
        let decision = report
            .decisions
            .iter()
            .find(|d| d.proposal_id == "python-version-20")
            .unwrap();
        assert_eq!(
            (decision.status.as_str(), decision.by_source.as_deref()),
            ("superseded", Some("app_bundle"))
        );
        assert!(
            report
                .effective
                .iter()
                .any(|e| e.tag.dimension == "runtime" && e.source == "python")
        );
        temp.close().unwrap();
    }

    #[test]
    fn framework_python_inside_python_app_keeps_runtime_work_and_version() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let venv = root.join(".venv/bin");
        std::fs::create_dir_all(&venv).unwrap();
        std::fs::write(root.join("sync_mail.py"), "").unwrap();
        let user = unsafe { libc::geteuid() };
        let process = |pid: u32, executable: &str, argv: &[&str]| Process {
            pid,
            ppid: Some(1),
            uid: Some(user),
            started_unix_us: Some(u64::from(pid)),
            name: "Python".into(),
            executable: Some(executable.into()),
            cwd: Some(root.clone()),
            resident_bytes: Some(1),
            command: Some(crate::model::ProcessCommand {
                argv: Some(args(argv)),
                ..Default::default()
            }),
            ..Default::default()
        };
        let venv_python = venv.join("python").to_string_lossy().into_owned();
        let framework = root
            .join(FRAMEWORK.trim_start_matches("/opt/homebrew/"))
            .to_string_lossy()
            .into_owned();
        patchlevel(
            Path::new(&framework).ancestors().nth(5).unwrap(),
            "3.14",
            "3.14.10",
        );
        let tree = crate::tree::build(vec![
            process(
                20,
                &framework,
                &["Python", "-u", "sync_mail.py", "--token", "hunter2"],
            ),
            process(21, &venv_python, &["python", "-c", "pass"]),
        ])
        .unwrap();
        let apps = crate::tree::select(&tree, "group:applications").unwrap();
        let row = crate::tree::select(&apps, "pid:20").unwrap();
        assert_eq!(row.display_name(), "sync_mail");
        let process = row.process.unwrap();
        let report = process.tag_report.as_ref().unwrap();
        for dimension in ["runtime", "work", "version"] {
            assert!(
                report
                    .effective
                    .iter()
                    .any(|e| e.tag.dimension == dimension && e.source == "python"),
                "{dimension}"
            );
        }
        assert!(
            report
                .decisions
                .iter()
                .all(|d| d.reason != "version_subject_mismatch")
        );
        assert_eq!(process.version.as_ref().unwrap().value, "3.14.10");
        let tags = crate::output::effective_tags_text(Some(report));
        assert!(
            tags.contains("#python") && tags.contains("work sync_mail (argv_script)"),
            "{tags}"
        );
        let venv_row = crate::tree::select(&tree, "pid:21").unwrap();
        assert_eq!(venv_row.display_name(), "Python");
        assert!(venv_row.process.unwrap().version.is_none());
        let snapshot = crate::model::Snapshot {
            schema: crate::model::JSON_SCHEMA_VERSION,
            extensions: vec![],
            captured_unix_ms: 1,
            sample_ms: 1,
            collection_ms: 1,
            system: Default::default(),
            scope: "group:all".into(),
            warnings: vec![],
            root: tree,
        };
        let markdown = crate::output::markdown(&snapshot);
        assert!(
            markdown.contains("- sync\\_mail pid:20 · ")
                || markdown.contains("- sync_mail pid:20 · "),
            "{markdown}"
        );
        let json = serde_json::to_value(&snapshot).unwrap();
        let schema: serde_json::Value = serde_json::from_str(crate::output::JSON_SCHEMA).unwrap();
        assert!(jsonschema::validator_for(&schema).unwrap().is_valid(&json));
        let text = json.to_string() + &markdown;
        for leaked in ["hunter2", "--token"] {
            assert!(!text.contains(leaked), "{leaked}");
        }
        temp.close().unwrap();
    }
}
