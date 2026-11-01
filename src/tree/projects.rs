use super::stable_ref;
use crate::model::{Attribution, AttributionSource, Diagnostic, Grouping, Node};
use std::{
    collections::{BTreeMap, HashMap},
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
};

const MAX_MARKER_BYTES: u64 = 8192;
const MANIFESTS: &[&str] = &[
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

#[derive(Clone, Debug)]
pub(crate) struct Project {
    pub root: PathBuf,
    checkout: Option<PathBuf>,
    manifest: Option<&'static str>,
}

impl Project {
    pub fn attribution(&self) -> Attribution {
        let source = if self.checkout.as_ref() == Some(&self.root) {
            AttributionSource::GitRoot
        } else {
            AttributionSource::Manifest
        };
        let mut value = Attribution::new(source, Some(self.root.clone()));
        value.checkout_path = self.checkout.clone();
        value.manifest = self.manifest.map(str::to_owned);
        value
    }

    pub fn grouping(&self) -> Grouping {
        if self.checkout.as_ref() == Some(&self.root) {
            Grouping::Checkout
        } else {
            Grouping::Project
        }
    }

    pub fn reference(&self) -> String {
        let namespace = if self.checkout.as_ref() == Some(&self.root) {
            "development"
        } else {
            "project"
        };
        stable_ref(namespace, &self.root.to_string_lossy())
    }

    pub fn evidence(&self) -> String {
        if self.checkout.as_ref() == Some(&self.root) {
            format!("Git root from working directory: {}", self.root.display())
        } else if let Some(manifest) = self.manifest {
            format!("project {manifest} {}", self.root.display())
        } else {
            format!("project {}", self.root.display())
        }
    }
}

#[derive(Default)]
pub(crate) struct Discovery {
    cache: HashMap<PathBuf, Option<Project>>,
}

impl Discovery {
    pub fn find(&mut self, cwd: &Path) -> Option<Project> {
        if let Some(project) = self.cache.get(cwd) {
            return project.clone();
        }
        let found = self.find_canonical(cwd);
        self.cache.insert(cwd.into(), found.clone());
        found
    }

    fn find_canonical(&mut self, cwd: &Path) -> Option<Project> {
        if !cwd.is_absolute() {
            return None;
        }
        let cwd = cwd.canonicalize().ok()?;
        if !cwd.is_dir() {
            return None;
        }
        let mut visited = Vec::new();
        let mut found = None;
        for path in cwd.ancestors() {
            if let Some(cached) = self.cache.get(path) {
                found = cached.clone();
                break;
            }
            let manifest = MANIFESTS
                .iter()
                .copied()
                .find(|name| path.join(name).is_file());
            let git = path.join(".git");
            if git.is_dir() || git.is_file() {
                found = Some(Project {
                    root: path.into(),
                    checkout: Some(path.into()),
                    manifest,
                });
                self.cache.insert(path.into(), found.clone());
                break;
            }
            visited.push((path.to_path_buf(), manifest));
        }
        for (path, manifest) in visited.into_iter().rev() {
            if manifest.is_some() {
                found = Some(Project {
                    root: path.clone(),
                    checkout: found.as_ref().and_then(|project| project.checkout.clone()),
                    manifest,
                });
            }
            self.cache.insert(path, found.clone());
        }
        found
    }
}

struct Checkout {
    common_dir: PathBuf,
    label: String,
}

fn marker(path: &Path) -> io::Result<String> {
    if !path.metadata()?.is_file() {
        return Err(io::Error::other("Git marker is not a regular file"));
    }
    let mut text = String::new();
    File::open(path)?
        .take(MAX_MARKER_BYTES + 1)
        .read_to_string(&mut text)?;
    if text.len() as u64 > MAX_MARKER_BYTES {
        return Err(io::Error::other("Git marker exceeds size limit"));
    }
    let text = text.trim_end_matches(['\r', '\n']);
    if text.is_empty() || text.contains(['\n', '\r', '\0']) {
        return Err(io::Error::other("Invalid Git marker"));
    }
    Ok(text.into())
}

fn directory(path: PathBuf) -> io::Result<PathBuf> {
    let path = path.canonicalize()?;
    if !path.is_dir() {
        return Err(io::Error::other("Git directory is not a directory"));
    }
    Ok(path)
}

fn checkout(root: &Path) -> io::Result<Checkout> {
    let git = root.join(".git");
    let git_dir = if git.is_dir() {
        directory(git)?
    } else {
        let contents = marker(&git)?;
        let path = contents
            .strip_prefix("gitdir: ")
            .filter(|path| !path.is_empty())
            .ok_or_else(|| io::Error::other("Invalid gitdir marker"))?;
        directory(root.join(path))?
    };
    let common_dir = match marker(&git_dir.join("commondir")) {
        Ok(path) => directory(git_dir.join(path))?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => git_dir.clone(),
        Err(error) => return Err(error),
    };
    let label = if common_dir == git_dir {
        "main checkout".into()
    } else {
        format!(
            "worktree {}",
            git_dir.file_name().unwrap_or_default().to_string_lossy()
        )
    };
    Ok(Checkout { common_dir, label })
}

pub(super) fn group(projects: Vec<(Project, Node)>) -> Vec<Node> {
    let mut by_checkout: BTreeMap<PathBuf, Vec<(Project, Node)>> = BTreeMap::new();
    let mut groups = Vec::new();
    for (project, node) in projects {
        if let Some(checkout) = &project.checkout {
            by_checkout
                .entry(checkout.clone())
                .or_default()
                .push((project, node));
        } else {
            groups.push(node);
        }
    }
    let mut checkouts = Vec::new();
    for (path, mut projects) in by_checkout {
        let mut checkout = if let Some(index) = projects
            .iter()
            .position(|(project, _)| project.root == path)
        {
            projects.remove(index).1
        } else {
            Node::group(
                stable_ref("development", &path.to_string_lossy()),
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                vec![format!(
                    "Git root from working directory: {}",
                    path.display()
                )],
                Grouping::Checkout,
            )
        };
        if checkout.attribution.is_none() {
            let mut attribution = Attribution::new(AttributionSource::GitRoot, Some(path.clone()));
            attribution.checkout_path = Some(path.clone());
            checkout.attribution = Some(attribution);
        }
        for (_, project) in projects {
            checkout.totals.add(&project.totals);
            checkout.children.push(project);
        }
        checkouts.push((path, checkout));
    }
    groups.extend(group_checkouts(checkouts));
    groups
}

fn group_checkouts(checkouts: Vec<(PathBuf, Node)>) -> Vec<Node> {
    let mut repositories: BTreeMap<PathBuf, Vec<(String, Node)>> = BTreeMap::new();
    let mut groups = Vec::new();
    for (path, mut node) in checkouts {
        match checkout(&path) {
            Ok(checkout) => repositories
                .entry(checkout.common_dir)
                .or_default()
                .push((checkout.label, node)),
            Err(error) => {
                node.evidence
                    .push(format!("Repository metadata unavailable: {error}"));
                node.diagnostics
                    .push(Diagnostic::io("repository_metadata", &error));
                groups.push(node);
            }
        }
    }
    for (common, checkouts) in repositories {
        if checkouts.len() == 1 {
            groups.extend(checkouts.into_iter().map(|(_, node)| node));
            continue;
        }
        let name_path = if common.file_name().is_some_and(|name| name == ".git") {
            common.parent().unwrap_or(&common)
        } else {
            &common
        };
        let name = name_path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .trim_end_matches(".git")
            .to_owned();
        let mut repository = Node::group(
            stable_ref("repository", &common.to_string_lossy()),
            name,
            vec![format!("repo {}", common.display())],
            Grouping::Repo,
        );
        repository.attribution = Some(Attribution::new(
            AttributionSource::GitCommonDir,
            Some(common),
        ));
        for (label, mut node) in checkouts {
            node.name = label;
            repository.totals.add(&node.totals);
            repository.children.push(node);
        }
        groups.push(repository);
    }
    groups
}
