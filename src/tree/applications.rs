use super::stable_ref;
use crate::model::{Attribution, AttributionSource, Grouping, Node};
use plist::Value;
use std::{
    collections::BTreeMap,
    fs::File,
    io::{Cursor, Read},
    path::{Path, PathBuf},
};

const MAX_PLIST_BYTES: u64 = 1024 * 1024;

struct Bundle {
    identifier: String,
    name: Option<String>,
    version: Option<String>,
}

fn metadata(path: &Path) -> Option<Bundle> {
    let path = path.join("Contents/Info.plist");
    let info = path.metadata().ok()?;
    if !info.is_file() || info.len() > MAX_PLIST_BYTES {
        return None;
    }
    let mut bytes = Vec::new();
    File::open(path)
        .ok()?
        .take(MAX_PLIST_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_PLIST_BYTES {
        return None;
    }
    let value = Value::from_reader(Cursor::new(bytes)).ok()?;
    let dictionary = value.as_dictionary()?;
    let string = |key: &str| {
        dictionary
            .get(key)?
            .as_string()
            .filter(|value| !value.trim().is_empty() && !value.chars().any(char::is_control))
            .map(str::to_owned)
    };
    let identifier = string("CFBundleIdentifier")?;
    if !identifier
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
    {
        return None;
    }
    Some(Bundle {
        identifier,
        name: string("CFBundleDisplayName").or_else(|| string("CFBundleName")),
        version: string("CFBundleShortVersionString"),
    })
}

pub(super) fn group(mut bundles: Vec<(PathBuf, Node)>) -> Vec<Node> {
    bundles.sort_by(|a, b| a.0.cmp(&b.0));
    let mut applications: BTreeMap<String, Vec<(Bundle, Node)>> = BTreeMap::new();
    let mut groups = Vec::new();
    for (path, mut node) in bundles {
        if let Some(bundle) = metadata(&path) {
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
    use crate::{model::Process, tree};

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
        let installed_ref = stable_ref("applications", &installed.to_string_lossy());
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
