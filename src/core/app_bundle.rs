use crate::extensions::{
    Capability, ClassificationProposal, Context, Contribution, ProcessIdentity, Request,
    host::{
        Facts, Module, bundles, no_settings,
        proposals::{identity_proposal, ownership},
    },
    tags::path_id,
};
use crate::model::Process;

pub(crate) const ID: &str = "app_bundle";

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
        proposals: proposals(
            facts,
            context
                .settings
                .get("core.bundle_version")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true),
        ),
    })
}

fn version(
    facts: &Facts<'_>,
    process: &Process,
    identity: ProcessIdentity,
) -> Option<ClassificationProposal> {
    let executable = process.executable.as_deref()?;
    let file = executable.file_name()?.to_str()?;
    let bundle = bundles::lookup(&facts.bundles, bundles::main_bundle(executable)?)?;
    let value = bundle.version.as_deref()?;
    if bundle.executable.as_deref() != Some(file) || file.len() > 128 || value.len() > 128 {
        return None;
    }
    Some(identity_proposal(
        identity,
        ID,
        "info_plist",
        "version",
        value,
        Some(file),
    ))
}

fn proposals(facts: &Facts<'_>, versions_enabled: bool) -> Vec<ClassificationProposal> {
    let mut versions = Vec::new();
    let mut proposals: Vec<_> = facts
        .processes
        .iter()
        .filter_map(|process| {
            let identity = ProcessIdentity::observed(process)?;
            let direct = &facts.direct[&process.pid];
            if direct.category != "applications" {
                return None;
            }
            let path = direct.attribution.as_ref().and_then(|a| a.path.clone())?;
            if let Some(version) = versions_enabled
                .then(|| version(facts, process, identity))
                .flatten()
            {
                versions.push(version);
            }
            Some(ownership(
                identity,
                ID,
                "app",
                path_id("app", &path),
                &direct.name,
                path,
                direct.clone(),
            ))
        })
        .collect();
    proposals.extend(versions);
    proposals
}
