use super::stable_ref;
use crate::extensions::host::bundles::{Bundle, Metadata, lookup};
use crate::model::{Attribution, AttributionSource, Grouping, Node};
use std::{collections::BTreeMap, path::PathBuf};

pub(super) fn group(
    mut bundles: Vec<(PathBuf, Node)>,
    metadata: &Metadata,
    versions_enabled: bool,
) -> Vec<Node> {
    bundles.sort_by(|a, b| a.0.cmp(&b.0));
    let mut applications: BTreeMap<String, Vec<(Bundle, Node)>> = BTreeMap::new();
    let mut groups = Vec::new();
    for (path, mut node) in bundles {
        if let Some(bundle) = lookup(metadata, &path).cloned() {
            if let Some(attribution) = &mut node.attribution {
                attribution.bundle_id = Some(bundle.identifier.clone());
            }
            applications
                .entry(bundle.identifier.clone())
                .or_default()
                .push((bundle, node));
        } else {
            groups.push(node);
        }
    }
    for (identifier, bundles) in applications {
        if bundles.len() == 1 {
            groups.extend(bundles.into_iter().map(|(_, node)| node));
            continue;
        }
        let name = bundles
            .iter()
            .find_map(|(bundle, _)| bundle.name.clone())
            .unwrap_or_else(|| bundles[0].1.name.clone());
        let mut application = Node::group(
            stable_ref("application", &identifier),
            name,
            vec![format!("bundle {identifier}")],
            Grouping::App,
        );
        let mut attribution = Attribution::new(AttributionSource::BundleId, None);
        attribution.bundle_id = Some(identifier);
        application.attribution = Some(attribution);
        for (bundle, mut node) in bundles {
            node.name = bundle
                .version
                .filter(|_| versions_enabled)
                .map_or_else(|| "bundle".into(), |version| format!("bundle {version}"));
            application.totals.add(&node.totals);
            application.children.push(node);
        }
        groups.push(application);
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::extensions::host::bundles::{MAX_PLIST_BYTES, PLIST_READS};
    use crate::{model::Process, tree};
    use plist::Value;
    use std::{fs::File, path::Path};

    fn fixture(path: &Path, identifier: &str, version: Option<&str>, binary: bool) {
        std::fs::create_dir_all(path.join("Contents")).unwrap();
        let mut dictionary = plist::Dictionary::new();
        dictionary.insert("CFBundleIdentifier".into(), identifier.into());
        dictionary.insert("CFBundleDisplayName".into(), "Example Browser".into());
        if let Some(version) = version {
            dictionary.insert("CFBundleShortVersionString".into(), version.into());
        }
        let value = Value::Dictionary(dictionary);
        let file = File::create(path.join("Contents/Info.plist")).unwrap();
        if binary {
            value.to_writer_binary(file).unwrap();
        } else {
            value.to_writer_xml(file).unwrap();
        }
    }

    fn process(pid: u32, ppid: u32, path: &Path, memory: Option<u64>) -> Process {
        Process {
            pid,
            ppid: Some(ppid),
            name: "example".into(),
            executable: Some(path.join("Contents/MacOS/example")),
            started_unix_us: Some(u64::from(pid)),
            uptime_seconds: Some(100 - u64::from(pid)),
            resident_bytes: memory,
            cpu_percent: Some(1.0),
            ..Process::default()
        }
    }

    fn info(path: &Path, entries: &[(&str, &str)]) {
        std::fs::create_dir_all(path.join("Contents/MacOS")).unwrap();
        let mut dictionary = plist::Dictionary::new();
        for (key, value) in entries {
            dictionary.insert((*key).into(), (*value).into());
        }
        Value::Dictionary(dictionary)
            .to_writer_xml(File::create(path.join("Contents/Info.plist")).unwrap())
            .unwrap();
    }

    #[test]
    fn bundle_versions_reach_main_programs_and_linked_helpers_but_not_embedded_tools() {
        let temp = tempfile::tempdir().unwrap();
        let browser = temp.path().join("9.9.9/Browser.app");
        let helper = browser.join("Contents/Frameworks/Helper.app");
        let unlinked = browser.join("Contents/Frameworks/Unlinked.app");
        info(
            &browser,
            &[
                ("CFBundleIdentifier", "org.example.browser"),
                ("CFBundleShortVersionString", "2.0"),
                ("CFBundleExecutable", "example"),
            ],
        );
        info(
            &helper,
            &[
                ("CFBundleIdentifier", "org.example.helper"),
                ("CFBundleShortVersionString", "3.1"),
                ("CFBundleExecutable", "Helper"),
            ],
        );
        info(
            &unlinked,
            &[
                ("CFBundleIdentifier", "org.example.unlinked"),
                ("CFBundleShortVersionString", "4.0"),
                ("CFBundleExecutable", "Different"),
            ],
        );
        let at = |pid: u32, executable: PathBuf| Process {
            executable: Some(executable),
            ..process(pid, 1, &browser, Some(1))
        };
        let processes = vec![
            process(10, 1, &browser, Some(1)),
            at(11, browser.join("Contents/Resources/bin/node")),
            at(12, helper.join("Contents/MacOS/Helper")),
            at(13, unlinked.join("Contents/MacOS/Unlinked")),
            at(14, browser.join("Contents/MacOS/other")),
        ];
        let reads = PLIST_READS.with(std::cell::Cell::get);
        let tree = tree::build(processes.clone()).unwrap();
        assert_eq!(PLIST_READS.with(std::cell::Cell::get) - reads, 3);
        let version = |pid: &str| {
            tree::select(&tree, pid)
                .unwrap()
                .process
                .unwrap()
                .version
                .map(|v| (v.subject, v.value, v.basis, v.source))
        };
        assert_eq!(
            version("pid:10"),
            Some((
                "example".into(),
                "2.0".into(),
                "info_plist".into(),
                "app_bundle".into()
            ))
        );
        assert_eq!(version("pid:12").map(|v| v.1), Some("3.1".into()));
        for pid in ["pid:11", "pid:13", "pid:14"] {
            assert_eq!(version(pid), None, "{pid}");
        }
        let main = tree::select(&tree, "pid:10").unwrap();
        assert_eq!(main.display_name(), "example");
        assert_eq!(tree.totals.processes, 5);
        let settings = crate::extensions::settings_registry()
            .unwrap()
            .resolve(
                &crate::settings::Config::default(),
                &[("core.bundle_version".into(), serde_json::Value::Bool(false))].into(),
            )
            .unwrap();
        let cancel = crate::snapshot::Cancellation::default();
        let without_versions = tree::build_with_providers(
            processes,
            &crate::extensions::Context::new(&settings, &cancel),
            &[],
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(&without_versions.totals).unwrap(),
            serde_json::to_value(&tree.totals).unwrap()
        );
        for pid in ["pid:10", "pid:12"] {
            let node = tree::select(&without_versions, pid).unwrap();
            assert!(node.process.as_ref().unwrap().version.is_none());
            assert!(
                node.process
                    .as_ref()
                    .unwrap()
                    .tag_report
                    .as_ref()
                    .unwrap()
                    .effective
                    .iter()
                    .any(|e| e.tag.dimension == "app")
            );
        }
        temp.close().unwrap();
    }

    #[test]
    fn bundle_identity_groups_copies_without_losing_paths_refs_or_totals() {
        let temp = tempfile::tempdir().unwrap();
        let installed = temp.path().join("installed/Browser.app");
        let copy = temp.path().join("temporary/Renamed.app.bundle");
        let other = temp.path().join("other/Browser.app");
        fixture(&installed, "org.example.browser", Some("2.0"), false);
        fixture(&copy, "org.example.browser", Some("1.0"), true);
        fixture(&other, "org.example.unrelated", None, true);
        let a = process(10, 1, &copy, Some(100));
        let b = process(11, 10, &installed, Some(200));
        let c = process(12, 11, &installed, None);
        let d = process(13, 1, &other, Some(400));
        let tree = tree::build(vec![a.clone(), b.clone(), c, d]).unwrap();
        let applications = tree::select(&tree, "group:applications").unwrap();
        assert_eq!(applications.children.len(), 2);
        assert_eq!(tree.totals.processes, 4);
        assert_eq!(tree.totals.resident_bytes, 700);
        assert_eq!(tree.totals.memory_unknown, 1);
        let reference = stable_ref("application", "org.example.browser");
        let mut family = tree::select(&tree, &reference).unwrap();
        assert_eq!(family.name, "Example Browser");
        assert_eq!(family.children.len(), 2);
        assert_eq!(family.totals.processes, 3);
        assert_eq!(family.totals.resident_bytes, 300);
        let attribution = serde_json::to_value(&family.attribution).unwrap();
        assert_eq!(attribution["source"], "bundle_id");
        assert_eq!(attribution["bundle_id"], "org.example.browser");
        assert!(attribution["path"].is_null());
        assert_eq!(family.totals.cpu_percent, 3.0);
        assert_eq!(family.totals.oldest_uptime_seconds, Some(90));
        let installed_ref = crate::extensions::tags::ownership_ref(
            "app",
            &crate::extensions::tags::path_id("app", &installed),
            &installed,
        );
        let installed_node = tree::select(&family, &installed_ref).unwrap();
        assert_eq!(installed_node.name, "bundle 2.0");
        assert_eq!(
            installed_node.attribution.as_ref().unwrap().path.as_ref(),
            Some(&installed)
        );
        assert_eq!(installed_node.children[0].reference, "pid:11");
        assert_eq!(installed_node.children[0].children[0].reference, "pid:12");
        assert_eq!(
            installed_node.children[0].process.as_ref().unwrap().ppid,
            Some(10)
        );
        assert!(
            installed_node
                .evidence
                .contains(&format!("Application bundle: {}", installed.display()))
        );
        assert!(family.children.iter().any(|node| node.name == "bundle 1.0"));
        tree::limit_depth(&mut family, 0);
        assert_eq!(family.omitted_children, 2);
        assert_eq!(family.totals.resident_bytes, 300);
        let settings = crate::extensions::settings_registry()
            .unwrap()
            .resolve(
                &crate::settings::Config::default(),
                &[("core.bundle_version".into(), serde_json::Value::Bool(false))].into(),
            )
            .unwrap();
        let cancel = crate::snapshot::Cancellation::default();
        let hidden = tree::build_with_providers(
            vec![a.clone(), b.clone()],
            &crate::extensions::Context::new(&settings, &cancel),
            &[],
        )
        .unwrap();
        let hidden_family = tree::select(&hidden, &reference).unwrap();
        assert_eq!(hidden_family.name, "Example Browser");
        assert_eq!(hidden_family.totals.processes, 2);
        assert!(
            hidden_family
                .children
                .iter()
                .all(|node| node.name == "bundle")
        );
        assert_eq!(
            tree::select(&hidden, &installed_ref)
                .unwrap()
                .attribution
                .as_ref()
                .unwrap()
                .path
                .as_ref(),
            Some(&installed)
        );
        let reversed = tree::build(vec![b.clone(), a]).unwrap();
        assert_eq!(
            tree::select(&reversed, &reference).unwrap().name,
            "Example Browser"
        );
        let single = tree::build(vec![b]).unwrap();
        assert!(tree::select(&single, &reference).is_err());
        assert_eq!(
            tree::select(&single, &installed_ref).unwrap().name,
            "Browser"
        );
        temp.close().unwrap();
    }

    #[test]
    fn unreadable_or_invalid_bundle_metadata_keeps_path_groups() {
        let temp = tempfile::tempdir().unwrap();
        let mut processes = Vec::new();
        for (index, contents) in [
            None,
            Some(b"not a plist".to_vec()),
            Some(b"<?xml version=\"1.0\"?><plist version=\"1.0\"><dict/></plist>".to_vec()),
            Some(vec![b' '; MAX_PLIST_BYTES as usize + 1]),
        ]
        .into_iter()
        .enumerate()
        {
            let path = temp.path().join(format!("{index}/Browser.app"));
            std::fs::create_dir_all(path.join("Contents")).unwrap();
            if let Some(contents) = contents {
                std::fs::write(path.join("Contents/Info.plist"), contents).unwrap();
            }
            processes.push(process(index as u32 + 10, 1, &path, Some(100)));
        }
        let invalid = temp.path().join("invalid/Browser.app");
        fixture(&invalid, "invalid identifier", None, false);
        processes.push(process(20, 1, &invalid, Some(100)));
        let tree = tree::build(processes).unwrap();
        let applications = tree::select(&tree, "group:applications").unwrap();
        assert_eq!(applications.children.len(), 5);
        assert_eq!(applications.totals.resident_bytes, 500);
        assert!(
            applications
                .children
                .iter()
                .all(|node| node.children[0].kind == "process")
        );
        temp.close().unwrap();
    }
}
