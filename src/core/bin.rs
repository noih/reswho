use crate::extensions::{
    Capability, ClassificationProposal, Context, Contribution, ProcessIdentity, Request,
    host::{Facts, Module, assignment, no_settings, proposals::ownership},
    tags::canonical_id,
};

pub(crate) const ID: &str = "bin";

pub(crate) static MODULE: Module = Module {
    registration: crate::extensions::host::no_registration,
    decorate: None,
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
            let assigned = if assignment::system_executable(process.executable.as_deref()) {
                assignment::system("System executable location")
            } else {
                assignment::unknown("No effective ownership tag")
            };
            let path = facts.executables.get(&process.pid).cloned()?;
            let name = process
                .executable
                .as_deref()
                .and_then(|p| p.file_name())
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            Some(ownership(
                identity,
                ID,
                "bin",
                canonical_id("bin", &path),
                &name,
                path,
                assigned,
            ))
        })
        .collect()
}
