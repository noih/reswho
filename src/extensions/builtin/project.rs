use crate::extensions::host::projects::Project;
use crate::extensions::{
    Capability, ClassificationProposal, Context, Contribution, ProcessIdentity, Request,
    host::{Facts, Module, no_settings, projects, proposals::ownership},
    tags::path_id,
};
use crate::model::{Attribution, AttributionSource, Grouping};
use std::{
    fs::File,
    io::{self, Read},
    path::{Path, PathBuf},
};

pub(crate) const ID: &str = "project";

pub(crate) static MODULE: Module = Module {
    summary: Some(summary),
    registration,
    decorate: Some(decorate),
    id: ID,
    capability: Capability::Classify,
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

fn proposals(facts: &Facts<'_>) -> Vec<ClassificationProposal> {
    facts
        .processes
        .iter()
        .filter_map(|process| {
            let identity = ProcessIdentity::observed(process)?;
            let assigned = assignment(projects::find(facts, process.cwd.as_deref()?)?);
            let path = assigned.attribution.as_ref().and_then(|a| a.path.clone())?;
            let mut proposal = ownership(
                identity,
                ID,
                "project",
                path_id("project", &path),
                &assigned.name.clone(),
                path,
                assigned,
            );
            if let crate::extensions::ClassificationEvidence::Tags { proposal } =
                &mut proposal.evidence
                && let crate::extensions::tags::Operation::Add { tag } = &mut proposal.operation
            {
                tag.basis = "cwd".into();
                if !tag.label.chars().any(|c| c.is_control() || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{2028}' | '\u{2029}')) {
                    tag.hint = Some(Box::new(crate::extensions::host::hints::value("project", &tag.label)));
                }
            }
            Some(proposal)
        })
        .collect()
}

fn registration() -> crate::extensions::placement::Registration {
    use crate::extensions::placement::*;
    Registration {
        categories: vec![Category::new("projects", "Project associations", 200)],
        ownership: vec![Ownership {
            grouping: crate::model::Grouping::Project,
            ..Ownership::new("project", "projects", 200)
        }],
    }
}

fn assignment(project: Project) -> crate::extensions::host::assignment::Assignment {
    crate::extensions::host::assignment::Assignment {
        group_key: None,
        category: "projects".into(),
        category_name: "Project associations".into(),
        key: project.root.to_string_lossy().into_owned(),
        name: project
            .root
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        evidence: project.evidence(),
        attribution: Some(project.attribution()),
        projection: Some(projection(&project)),
    }
}

fn projection(project: &Project) -> crate::extensions::placement::Projection {
    use crate::extensions::{
        placement::{Parent, Projection},
        tags::{ownership_ref, path_id},
    };
    let mut result = Projection {
        grouping: project.grouping(),
        parents: vec![],
    };
    if let Some(path) = &project.checkout {
        let mut attribution = Attribution::new(AttributionSource::GitRoot, Some(path.clone()));
        attribution.checkout_path = Some(path.clone());
        let mut parent = Parent {
            reference: ownership_ref("project", &path_id("project", path), path),
            merge: project.root == *path,
            collision_reference: Some(crate::extensions::host::stable_ref(
                "checkout",
                &path.to_string_lossy(),
            )),
            name: path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            grouping: Grouping::Checkout,
            evidence: vec![format!(
                "Git root from working directory: {}",
                path.display()
            )],
            attribution: Some(attribution),
            diagnostics: vec![],
            collapse_single: false,
            child_name: None,
        };
        match checkout(path) {
            Ok(checkout) => {
                let common = checkout.common_dir;
                result.parents.push(parent);
                result.parents.push(Parent {
                    merge: false,
                    collision_reference: None,
                    reference: crate::extensions::host::stable_ref(
                        "repository",
                        &common.to_string_lossy(),
                    ),
                    name: repository_name(&common),
                    grouping: Grouping::Repo,
                    evidence: vec![format!("repo {}", common.display())],
                    attribution: Some(Attribution::new(
                        AttributionSource::GitCommonDir,
                        Some(common),
                    )),
                    diagnostics: vec![],
                    collapse_single: true,
                    child_name: Some(checkout.label),
                });
            }
            Err(error) => {
                parent
                    .evidence
                    .push(format!("Repository metadata unavailable: {error}"));
                parent
                    .diagnostics
                    .push(crate::model::Diagnostic::io("repository_metadata", &error));
                result.parents.push(parent);
            }
        }
    }
    result
}

fn decorate(facts: &Facts<'_>, tag: &mut crate::extensions::tags::Tag) {
    if tag.dimension != "project" || tag.assignment.is_some() || tag.projection.is_some() {
        return;
    }
    if let Some(path) = &tag.path
        && let Some(project) = projects::find(facts, path).filter(|project| project.root == *path)
    {
        let mut assigned = assignment(project);
        let mut attribution = assigned.attribution.take().expect("project attribution");
        attribution.source = AttributionSource::Extension;
        assigned.attribution = Some(attribution);
        tag.projection = assigned.projection.clone().map(Box::new);
        tag.assignment = Some(Box::new(assigned));
    }
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

const MAX_MARKER_BYTES: u64 = 8192;
struct Checkout {
    pub(crate) common_dir: PathBuf,
    pub(crate) label: String,
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

fn repository_name(common: &Path) -> String {
    let name_path = if common.file_name().is_some_and(|name| name == ".git") {
        common.parent().unwrap_or(common)
    } else {
        common
    };
    name_path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .trim_end_matches(".git")
        .to_owned()
}

pub(super) fn summary(
    context: &crate::extensions::SummaryContext<'_>,
) -> Option<crate::extensions::Summary> {
    let hint = context.hints().find(|hint| {
        hint.kind == "project" && matches!(hint.basis.as_str(), "cwd" | "ancestor_cwd")
    })?;
    if let Some(path) = &hint.path
        && context.ancestors.iter().any(|ancestor| {
            matches!(
                ancestor.grouping,
                Some(Grouping::Project | Grouping::Checkout)
            ) && ancestor
                .attribution
                .as_ref()
                .and_then(|attribution| attribution.path.as_ref())
                == Some(path)
        })
    {
        return None;
    }
    let inherited = hint.basis == "ancestor_cwd";
    let mut summary = crate::extensions::Summary::new(
        if inherited {
            "Project via parent cwd"
        } else {
            "Project via cwd"
        },
        &hint.value,
        20,
        vec![hint.clone()],
    );
    summary.inherited = inherited;
    Some(summary)
}
