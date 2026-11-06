use crate::extensions::{
    Capability, ClassificationProposal, Context, Contribution, ProcessIdentity, Request,
    host::{Facts, Module, bundles::app_bundle, no_settings, proposals::identity_proposal},
};
use std::path::Path;

pub(crate) const ID: &str = "versioned_install";

pub(crate) static MODULE: Module = Module {
    summary: None,
    registration: crate::extensions::host::no_registration,
    decorate: None,
    id: ID,
    capabilities: &[Capability::Classify],
    execute,
    settings: no_settings,
};

fn execute(_: &Context<'_>, request: Request<'_>) -> anyhow::Result<Contribution> {
    let Request::Classify(facts) = request else {
        anyhow::bail!("unsupported extension request");
    };
    Ok(Contribution::Classify {
        proposals: proposals(facts),
    })
}

fn version_file(name: &str) -> bool {
    let digits = name.strip_prefix(['v', 'V']).unwrap_or(name);
    digits.starts_with(|c: char| c.is_ascii_digit())
        && name.contains('.')
        && name.len() <= 128
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '+' | '_'))
}

fn install(executable: &Path) -> Option<(String, String)> {
    let version = executable.file_name()?.to_str()?;
    let versions = executable.parent()?;
    let tool = versions
        .parent()?
        .file_name()?
        .to_str()?
        .trim_start_matches('.');
    (version_file(version)
        && app_bundle(executable).is_none()
        && versions.file_name()? == "versions"
        && !tool.is_empty()
        && tool.len() <= 128
        && !tool.chars().any(char::is_control))
    .then(|| (tool.to_owned(), version.to_owned()))
}

fn proposals(facts: &Facts<'_>) -> Vec<ClassificationProposal> {
    facts
        .processes
        .iter()
        .filter_map(|process| {
            let identity = ProcessIdentity::observed(process)?;
            let (tool, version) = install(facts.executables.get(&process.pid)?)?;
            Some([
                identity_proposal(identity, ID, "executable", "name", &tool, None),
                identity_proposal(identity, ID, "executable", "version", &version, Some(&tool)),
            ])
        })
        .flatten()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_executables_that_are_the_version_file_qualify() {
        for (path, expected) in [
            (
                "/Users/u/.local/share/claude/versions/2.1.316",
                Some(("claude", "2.1.316")),
            ),
            ("/opt/tool/versions/v1.2.3", Some(("tool", "v1.2.3"))),
            ("/opt/tool/versions/2024.1", Some(("tool", "2024.1"))),
            ("/opt/tool/versions/1.2.3.4", Some(("tool", "1.2.3.4"))),
            (
                "/opt/.hidden/versions/1.0.0-rc.1",
                Some(("hidden", "1.0.0-rc.1")),
            ),
            ("/Users/u/.codegraph/versions/v1.6.2/node", None),
            (
                "/Users/u/.local/share/fnm/node-versions/v24.18.0/installation/bin/node",
                None,
            ),
            (
                "/opt/homebrew/Cellar/rust/1.99.1/libexec/rust-analyzer-proc-macro-srv",
                None,
            ),
            (
                "/Users/u/.codex/packages/app-server-daemon/releases/0.166.0/bin/codex",
                None,
            ),
            ("/opt/tool/releases/1.2.3", None),
            ("/opt/tool/1.2.3", None),
            ("/opt/tool/versions/latest", None),
            ("/opt/tool/versions/12345", None),
            ("/opt/tool/versions/.1.2", None),
            ("/opt/tool/versions/1.2.3 copy", None),
            ("/versions/1.2.3", None),
            ("/Apps/Tool.app/Contents/MacOS/versions/1.2.3", None),
        ] {
            let found = install(Path::new(path));
            assert_eq!(
                found.as_ref().map(|(t, v)| (t.as_str(), v.as_str())),
                expected,
                "{path}"
            );
        }
    }

    #[test]
    fn builtin_names_and_versions_versioned_installs_everywhere_and_skips_foreign_versions() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir(temp.path().join(".git")).unwrap();
        let claude = "/Users/u/.local/share/claude/versions/2.1.316";
        let process = |pid: u32, name: &str, executable: &str| crate::model::Process {
            pid,
            ppid: Some(1),
            uid: Some(501),
            started_unix_us: Some(u64::from(pid)),
            name: name.into(),
            executable: Some(executable.into()),
            resident_bytes: Some(1),
            ..Default::default()
        };
        let mut in_project = process(50, "2.1.316", claude);
        in_project.cwd = Some(temp.path().into());
        let tree = crate::tree::build(vec![
            in_project,
            process(51, "2.1.316", claude),
            process(52, "2.1.316", claude),
            process(53, "node", "/Users/u/.codegraph/versions/v1.6.2/node"),
        ])
        .unwrap();
        let projects = crate::tree::select(&tree, "group:projects").unwrap();
        let row = crate::tree::select(&projects, "pid:50").unwrap();
        assert_eq!(row.display_name(), "claude 2.1.316");
        let process = row.process.unwrap();
        assert_eq!(process.original_name.as_deref(), Some("2.1.316"));
        assert_eq!(process.name_source.unwrap().source, "versioned_install");
        let version = process.version.unwrap();
        assert_eq!(
            (
                version.subject.as_str(),
                version.value.as_str(),
                version.source.as_str()
            ),
            ("claude", "2.1.316", "versioned_install")
        );
        let report = process.tag_report.unwrap();
        for dimension in ["name", "version"] {
            assert!(
                report
                    .effective
                    .iter()
                    .any(|e| e.tag.dimension == dimension && e.source == "versioned_install")
            );
        }
        let unknown = crate::tree::select(&tree, "group:unclassified").unwrap();
        let bin = unknown
            .children
            .iter()
            .find(|n| n.totals.processes == 2)
            .unwrap();
        assert_eq!(bin.display_name(), "claude 2.1.316");
        let node = crate::tree::select(&tree, "pid:53").unwrap();
        assert_eq!(node.display_name(), "node");
        let node = node.process.unwrap();
        assert!(node.version.is_none() && node.original_name.is_none());
        assert!(
            node.tag_report
                .unwrap()
                .proposals
                .iter()
                .all(|r| r.source != "versioned_install")
        );
        assert_eq!(tree.totals.processes, 4);
        temp.close().unwrap();
    }
}
