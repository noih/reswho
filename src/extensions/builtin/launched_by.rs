use crate::extensions::{
    Capability, ClassificationEvidence, ClassificationProposal, Context, Contribution,
    ProcessIdentity, Request,
    host::{Facts, Module, bundles, no_registration, no_settings, proposals::label},
    tags::{Operation, ProcessEvidence, Proposal, Tag, path_id},
};
use crate::model::Process;
use std::collections::HashSet;

pub(crate) const ID: &str = "launched_by";
const DIMENSION: &str = "extensions.launched_by.app";

pub(crate) static MODULE: Module = Module {
    summary: None,
    registration: no_registration,
    decorate: None,
    id: ID,
    capability: Capability::Classify,
    execute,
    settings: no_settings,
};

fn proposal(facts: &Facts<'_>, process: &Process) -> Option<ClassificationProposal> {
    let identity = ProcessIdentity::observed(process)?;
    let mut current = process;
    let mut visited = HashSet::new();
    let mut chain = Vec::new();
    loop {
        if !visited.insert(current.pid) {
            return None;
        }
        let executable = current.executable.as_ref().filter(|p| p.is_absolute())?;
        chain.push(ProcessEvidence {
            identity: ProcessIdentity::observed(current)?,
            executable: executable.clone(),
        });
        if current.pid != process.pid
            && let Some(path) = bundles::app_bundle(executable)
        {
            let name = path.file_name()?.to_string_lossy();
            let name = name.trim_end_matches(".bundle").trim_end_matches(".app");
            let tag = Tag {
                hint: None,
                dimension: DIMENSION.into(),
                id: path_id(DIMENSION, &path),
                label: label(&format!("Launched by {name}")),
                path: Some(path),
                basis: "verified_ancestry".into(),
                subject: None,
                value: None,
                projection: None,
                host_resource: None,
                assignment: None,
                supplementary_to: Some("app".into()),
                evidence_chain: chain,
            };
            return Some(ClassificationProposal {
                identity,
                evidence: ClassificationEvidence::Tags {
                    proposal: Proposal {
                        id: format!("{ID}-{}", process.pid),
                        identity,
                        operation: Operation::Add { tag },
                    },
                },
            });
        }
        let parent = *facts.by_pid.get(&current.ppid.filter(|pid| *pid > 1)?)?;
        if current.uid? != parent.uid? || parent.started_unix_us? > current.started_unix_us? {
            return None;
        }
        current = parent;
    }
}

fn execute(_: &Context<'_>, request: Request<'_>) -> anyhow::Result<Contribution> {
    let Request::Classify(facts) = request else {
        anyhow::bail!("unsupported extension request");
    };
    Ok(Contribution::Classify {
        proposals: facts
            .processes
            .iter()
            .filter_map(|p| proposal(facts, p))
            .collect(),
    })
}

#[cfg(test)]
pub(crate) mod tests;
