use super::{
    docker::Inspect,
    model::{Container, Project, ProjectSource},
};
use crate::tree::projects::Discovery;
use std::{
    collections::BTreeSet,
    path::{Component, Path, PathBuf},
};

fn clean_absolute(path: &Path) -> bool {
    path.is_absolute()
        && !path
            .components()
            .any(|part| matches!(part, Component::ParentDir))
}

fn project_path(path: &Path, discovery: &mut Discovery) -> Option<PathBuf> {
    if !clean_absolute(path) {
        return None;
    }
    let directory = if path.is_file() { path.parent()? } else { path };
    discovery.find(directory).map(|project| project.root)
}

pub(super) fn classify(
    container: &mut Container,
    inspect: &Inspect,
    local: bool,
    discovery: &mut Discovery,
) {
    container.project = None;
    let candidates: BTreeSet<_> = if local {
        container
            .mounts
            .iter()
            .filter(|mount| mount.kind == "bind")
            .filter_map(|mount| project_path(&mount.source, discovery))
            .collect()
    } else {
        BTreeSet::new()
    };
    container.candidates = candidates.iter().cloned().collect();
    if let Some(name) = inspect
        .compose_project
        .as_ref()
        .filter(|value| !value.is_empty())
    {
        container.project = Some(Project {
            name: name.clone(),
            path: inspect
                .compose_workdir
                .as_ref()
                .filter(|path| !path.is_empty())
                .map(PathBuf::from),
            source: ProjectSource::Compose,
        });
        return;
    }
    if !local {
        return;
    }
    let mapped = inspect
        .workdir
        .as_deref()
        .map(Path::new)
        .filter(|path| clean_absolute(path))
        .and_then(|cwd| {
            let mount = container
                .mounts
                .iter()
                .filter(|mount| {
                    clean_absolute(&mount.destination) && cwd.starts_with(&mount.destination)
                })
                .max_by_key(|mount| mount.destination.components().count())?;
            if mount.kind != "bind" {
                return None;
            }
            project_path(
                &mount
                    .source
                    .join(cwd.strip_prefix(&mount.destination).ok()?),
                discovery,
            )
        });
    let inferred = mapped
        .map(|path| (path, ProjectSource::BindWorkdir))
        .or_else(|| {
            if candidates.len() == 1 {
                candidates
                    .into_iter()
                    .next()
                    .map(|path| (path, ProjectSource::BindProject))
            } else {
                None
            }
        });
    if let Some((path, source)) = inferred {
        container.project = Some(Project {
            name: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            path: Some(path),
            source,
        });
    }
}
