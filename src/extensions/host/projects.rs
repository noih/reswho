use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

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
    pub(crate) checkout: Option<PathBuf>,
    pub(crate) manifest: Option<&'static str>,
}

#[cfg_attr(not(test), derive(Default))]
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

pub(crate) fn find(facts: &super::Facts<'_>, path: &Path) -> Option<Project> {
    facts.cached::<Discovery, _>("project.discovery", |discovery| discovery.find(path))
}

#[cfg(test)]
pub(crate) static DISCOVERIES: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);
#[cfg(test)]
impl Default for Discovery {
    fn default() -> Self {
        #[cfg(test)]
        DISCOVERIES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Self {
            cache: HashMap::new(),
        }
    }
}
