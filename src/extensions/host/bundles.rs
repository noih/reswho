use plist::Value;
use std::{
    collections::BTreeMap,
    fs::File,
    io::{Cursor, Read},
    path::{Path, PathBuf},
};

pub(crate) const MAX_PLIST_BYTES: u64 = 1024 * 1024;

#[derive(Clone)]
pub(crate) struct Bundle {
    pub(crate) identifier: String,
    pub(crate) name: Option<String>,
    pub(crate) version: Option<String>,
    pub(crate) executable: Option<String>,
}

pub(crate) type Metadata = BTreeMap<PathBuf, Option<Bundle>>;

pub(crate) fn main_bundle(executable: &Path) -> Option<&Path> {
    if !executable.is_absolute() {
        return None;
    }
    let macos = executable.parent()?;
    let contents = macos.parent()?;
    let bundle = contents.parent()?;
    let name = bundle.file_name()?.to_str()?;
    (macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && (name.ends_with(".app") || name.ends_with(".app.bundle")))
    .then_some(bundle)
}

pub(crate) fn read_metadata(paths: impl IntoIterator<Item = PathBuf>) -> Metadata {
    let mut metadata = Metadata::new();
    for path in paths {
        let key = path.canonicalize().unwrap_or(path);
        if let std::collections::btree_map::Entry::Vacant(entry) = metadata.entry(key) {
            let bundle = read(entry.key());
            entry.insert(bundle);
        }
    }
    metadata
}

pub(crate) fn lookup<'a>(metadata: &'a Metadata, path: &Path) -> Option<&'a Bundle> {
    metadata
        .get(path)
        .or_else(|| metadata.get(&path.canonicalize().ok()?))?
        .as_ref()
}

#[cfg(test)]
thread_local! {
    pub(crate) static PLIST_READS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

fn read(path: &Path) -> Option<Bundle> {
    #[cfg(test)]
    PLIST_READS.with(|count| count.set(count.get() + 1));
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
        executable: string("CFBundleExecutable"),
    })
}

pub(crate) fn app_bundle(path: &Path) -> Option<PathBuf> {
    if !path.is_absolute() {
        return None;
    }
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
