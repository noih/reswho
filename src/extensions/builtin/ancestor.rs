use crate::extensions::{
    Capability, ClassificationProposal, Context, Contribution, ProcessIdentity, Request,
    host::{Facts, Module, assignment::Assignment, no_settings, proposals::ownership},
    tags::path_id,
};
use crate::model::Process;
use std::collections::{HashMap, HashSet};

pub(crate) const ID: &str = "ancestor";

pub(crate) static MODULE: Module = Module {
    summary: None,
    registration: crate::extensions::host::no_registration,
    decorate: None,
    id: ID,
    capability: Capability::Classify,
    execute,
    settings: no_settings,
};

fn execute(context: &Context<'_>, request: Request<'_>) -> anyhow::Result<Contribution> {
    let Request::Classify(facts) = request else {
        anyhow::bail!("unsupported extension request");
    };
    Ok(Contribution::Classify {
        proposals: proposals(facts, !context.settings.is_disabled("project")),
    })
}

fn inherited<'a>(
    process: &Process,
    processes: &HashMap<u32, &Process>,
    assignments: &'a HashMap<u32, Assignment>,
) -> Option<(u32, &'a Assignment)> {
    let mut current = process;
    let mut visited = HashSet::from([process.pid]);
    for _ in 0..128 {
        let pid = current.ppid.filter(|pid| *pid > 1)?;
        if !visited.insert(pid) {
            return None;
        }
        let parent = *processes.get(&pid)?;
        if current.uid? != parent.uid?
            || parent.started_unix_us? > current.started_unix_us?
            || current.cwd.as_ref()? != parent.cwd.as_ref()?
        {
            return None;
        }
        let assignment = assignments.get(&pid)?;
        match assignment.category.as_str() {
            "applications" => return Some((pid, assignment)),
            "unclassified" => current = parent,
            _ => return None,
        }
    }
    None
}

fn proposals(facts: &Facts<'_>, projects_enabled: bool) -> Vec<ClassificationProposal> {
    facts
        .processes
        .iter()
        .filter_map(|process| {
            let identity = ProcessIdentity::observed(process)?;
            let project = projects_enabled && process.cwd.as_deref().and_then(|path|crate::extensions::host::projects::find(facts,path)).is_some();
            if facts.direct[&process.pid].category != "unclassified" || project {
                return None;
            }
            let (pid, inherited) = inherited(process, &facts.by_pid, &facts.direct)?;
            let mut inherited = inherited.clone();
            if let Some(attribution) = &mut inherited.attribution {
                attribution.inferred = true;
                attribution.via_pid = Some(pid);
            }
            inherited.evidence = format!(
                "Inferred from ancestor pid:{pid}: {}; same UID and working directory along observed parent chain",
                inherited.evidence
            );
            let dimension = "app";
            let path = inherited.attribution.as_ref().and_then(|a| a.path.clone())?;
            Some(ownership(
                identity,
                ID,
                dimension,
                path_id(dimension, &path),
                &inherited.name.clone(),
                path,
                inherited,
            ))
        })
        .collect()
}
