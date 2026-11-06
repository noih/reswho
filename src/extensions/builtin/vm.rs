use crate::{
    extensions::{
        Capability, ClassificationEvidence, ClassificationProposal, Context, Contribution,
        ProcessIdentity, Request,
        host::{
            Facts, Module,
            assignment::Assignment,
            no_settings, proposals,
            vm::{self, Probe},
        },
        placement::{Ownership, Registration},
        tags::{Operation, ProcessEvidence, ownership_key, path_id},
    },
    model::{Attribution, AttributionSource, Diagnostic, Grouping, HostResource, Process},
};
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

pub(crate) const ID: &str = "vm";

pub(crate) static MODULE: Module = Module {
    summary: None,
    registration,
    decorate: None,
    id: ID,
    capabilities: &[Capability::Classify],
    execute,
    settings: no_settings,
};

fn registration() -> Registration {
    Registration {
        categories: vec![],
        ownership: [
            ("extensions.vm.app", "applications", Grouping::App),
            ("extensions.vm.helper", "unclassified", Grouping::Bin),
            ("extensions.vm.system", "system", Grouping::System),
        ]
        .into_iter()
        .map(|(dimension, category, grouping)| Ownership {
            dimension: dimension.into(),
            category: category.into(),
            priority: 50,
            grouping,
        })
        .collect(),
    }
}

type Owner = (PathBuf, Option<PathBuf>, String);

fn helper(path: &Path) -> Option<(&'static str, Owner)> {
    if let Some(owner) = crate::containers::docker::vm_helper(path) {
        return Some(("docker", owner));
    }
    #[cfg(target_os = "macos")]
    if let Some(owner) = crate::containers::apple::vm_helper(path) {
        return Some(("apple", owner));
    }
    None
}

fn disk(path: &Path, home: &Path) -> Option<String> {
    if let Some(scope) = crate::containers::docker::vm_disk(path, home) {
        return Some(scope);
    }
    #[cfg(target_os = "macos")]
    if let Some(scope) = crate::containers::apple::vm_disk(path, home) {
        return Some(scope);
    }
    None
}

fn owner(runtime: &str, facts: &Facts<'_>, process: &Process) -> Owner {
    for parent in facts.processes {
        if parent.uid == process.uid
            && parent
                .started_unix_us
                .is_some_and(|s| Some(s) <= process.started_unix_us)
            && let Some((source, owner)) = parent.executable.as_deref().and_then(helper)
            && source == runtime
        {
            return owner;
        }
    }
    if runtime == "docker" {
        let app = PathBuf::from("/Applications/Docker.app");
        if app.is_dir() {
            return (app.clone(), Some(app), "Docker".into());
        }
        (
            PathBuf::from("/Applications/Docker.app/Contents/MacOS/com.docker.backend"),
            None,
            "Docker".into(),
        )
    } else {
        (
            PathBuf::from(
                "/usr/local/libexec/container/plugins/container-runtime-linux/bin/container-runtime-linux",
            ),
            None,
            "Apple Container".into(),
        )
    }
}

fn proposal(
    facts: &Facts<'_>,
    process: &Process,
    probe: Probe,
    responsible: Option<&Process>,
    home: &Path,
) -> Option<ClassificationProposal> {
    let identity = ProcessIdentity::observed(process)?;
    let executable = process.executable.clone()?;
    let disks: BTreeSet<_> = probe
        .paths
        .into_iter()
        .take(64)
        .filter_map(|path| disk(&path, home).map(|scope| (scope, path)))
        .collect();
    let known = responsible
        .and_then(|p| p.executable.as_deref())
        .and_then(helper);
    let disk_runtime = disks
        .first()
        .map(|(scope, _)| scope.split(':').next().unwrap_or(scope));
    let conflict = disks.len() > 1
        || disk_runtime
            .zip(known.as_ref().map(|(r, _)| *r))
            .is_some_and(|(a, b)| a != b);
    let runtime = if conflict {
        None
    } else {
        disk_runtime.or_else(|| known.as_ref().map(|(r, _)| *r))
    };
    let confidence = if conflict {
        "conflict"
    } else if disk_runtime.is_some() && known.is_some() {
        "inferred"
    } else if runtime.is_some() {
        "partial"
    } else if responsible.is_some() {
        "unattributed"
    } else {
        "possible"
    };
    let scope = if conflict {
        None
    } else {
        disks
            .first()
            .map(|(scope, _)| scope.clone())
            .or_else(|| (runtime == Some("docker")).then(|| "docker".into()))
    };
    let evidence: Vec<_> = disks
        .iter()
        .map(|(_, path)| format!("Disk: {}", path.display()))
        .chain(
            probe
                .limited
                .then(|| "Only the first 64 file descriptors were inspected".into()),
        )
        .collect();
    let mut diagnostics = vec![];
    if conflict {
        diagnostics.push(Diagnostic::new(
            "invalid_data",
            "vm.conflict",
            "Conflicting runtime evidence; attribution withheld",
        ));
    } else if runtime.is_none() {
        diagnostics.push(Diagnostic::new(
            "unavailable",
            "vm.attribution",
            if responsible.is_some() {
                "Responsible process is not a supported runtime"
            } else {
                "VM evidence unavailable or not verified"
            },
        ));
    }
    let (dimension, path, name, mut assigned, group_key) = if let Some(runtime) = runtime {
        let (path, app, name) = known
            .as_ref()
            .filter(|(r, _)| *r == runtime)
            .map(|(_, o)| o.clone())
            .unwrap_or_else(|| owner(runtime, facts, process));
        let category = if app.is_some() {
            "applications"
        } else {
            "unclassified"
        };
        let base = if app.is_some() { "app" } else { "bin" };
        let dimension = if app.is_some() {
            "extensions.vm.app"
        } else {
            "extensions.vm.helper"
        };
        let mut attribution = Attribution::new(
            if app.is_some() {
                AttributionSource::AppBundle
            } else {
                AttributionSource::Executable
            },
            Some(path.clone()),
        );
        attribution.inferred = true;
        attribution.via_pid = responsible.map(|p| p.pid);
        let assigned = Assignment {
            group_key: None,
            category: category.into(),
            category_name: if app.is_some() {
                "Applications"
            } else {
                "Unclassified"
            }
            .into(),
            key: String::new(),
            name: name.clone(),
            evidence: format!("Virtual machine: {name} ({confidence})"),
            projection: None,
            attribution: Some(attribution),
        };
        let key = ownership_key(base, &path_id(base, &path), &path);
        let mut assigned = assigned;
        assigned.key = if app.is_some() {
            key.clone()
        } else {
            crate::extensions::host::assignment::unknown("Runtime helper location").key
        };
        (dimension, path, name, assigned, Some(key))
    } else {
        let name = if confidence == "possible" {
            "System possible VM"
        } else {
            "System virtual machine (unattributed)"
        }
        .to_owned();
        let mut assigned = crate::extensions::host::assignment::system(&name);
        assigned.name = name.clone();
        ("extensions.vm.system", executable, name, assigned, None)
    };
    let label = format!("{name}{}", if confidence == "partial" { " ~" } else { "" });
    assigned.name = label.clone();
    let mut proposal = proposals::ownership(
        identity,
        ID,
        dimension,
        path_id(dimension, &path),
        &label,
        path,
        assigned,
    );
    if let ClassificationEvidence::Tags { proposal: inner } = &mut proposal.evidence
        && let Operation::Add { tag } = &mut inner.operation
    {
        if let Some(assignment) = &mut tag.assignment {
            assignment.group_key = group_key;
        }
        tag.host_resource = Some(Box::new(HostResource {
            link_key: scope.clone(),
            kind: "virtual_machine".into(),
            scope,
            label,
            confidence: confidence.into(),
            diagnostics,
            responsible: responsible.and_then(|p| {
                Some(ProcessEvidence {
                    identity: ProcessIdentity::observed(p)?,
                    executable: p.executable.clone()?,
                })
            }),
            evidence,
            configuration: None,
        }));
    }
    Some(proposal)
}

fn execute(context: &Context<'_>, request: Request<'_>) -> anyhow::Result<Contribution> {
    let Request::Classify(facts) = request else {
        anyhow::bail!("unsupported extension request")
    };
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    let mut proposals = vec![];
    for process in facts.processes.iter().filter(|p| vm::candidate(p)) {
        context.check()?;
        let probe = vm::probe(process);
        let responsible = vm::responsible(
            process,
            probe.responsible,
            &facts.by_pid,
            crate::collect::verify_identity,
        );
        if let Some(proposal) = proposal(facts, process, probe, responsible, &home) {
            let label = if let ClassificationEvidence::Tags { proposal } = &proposal.evidence
                && let Operation::Add { tag } = &proposal.operation
            {
                tag.label.clone()
            } else {
                String::new()
            };
            proposals.push(proposals::identity_proposal(
                ProcessIdentity::observed(process).expect("proposal identity"),
                ID,
                "observed",
                "name",
                &format!("VM: {label}"),
                None,
            ));
            proposals.push(proposal);
        }
        context.check()?;
    }
    Ok(Contribution::Classify { proposals })
}

#[cfg(test)]
pub(crate) mod tests;
