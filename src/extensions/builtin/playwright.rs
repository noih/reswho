use crate::extensions::{
    Capability, ClassificationProposal, Context, Contribution, Request,
    host::{
        Facts, Module, hints, no_registration, no_settings, projects,
        runtime::{Argv, Shared, resolve},
    },
};
use crate::model::Process;
use std::collections::HashMap;

pub(crate) static MODULE: Module = Module {
    summary: Some(summary),
    registration: no_registration,
    decorate: None,
    id: "playwright",
    capability: Capability::Classify,
    execute,
    settings: no_settings,
};

fn session(process: &Process, reader: &mut impl Argv) -> Option<String> {
    if process.uid != Some(unsafe { libc::geteuid() }) {
        return None;
    }
    let executable = process.executable.as_deref()?;
    if !executable
        .file_name()
        .is_some_and(|name| name == "node" || name == "nodejs")
    {
        return None;
    }
    let argv = reader.read(process)?;
    let entry = super::node::entry(argv.clone(), executable, process.cwd.as_deref())?;
    if !entry.ends_with("playwright-core/lib/entry/cliDaemon.js") {
        return None;
    }
    let crate::extensions::host::files::Trusted::Bytes(bytes) =
        crate::extensions::host::files::read(
            &entry.ancestors().nth(3)?.join("package.json"),
            process.uid,
        )
    else {
        return None;
    };
    let package: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    if package["name"] != "playwright-core" || !package["version"].as_str()?.starts_with("1.64.") {
        return None;
    }

    let index = argv
        .iter()
        .position(|arg| resolve(arg, process.cwd.as_deref()).as_ref() == Some(&entry))?;
    let name = argv.get(index + 1)?;
    (!name.is_empty()
        && name.len() <= 128
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c))
        && !crate::extensions::host::runtime::suspicious(name))
    .then(|| name.clone())
}

fn proposals(
    facts: &Facts<'_>,
    reader: &mut impl Argv,
    project_enabled: bool,
) -> Vec<ClassificationProposal> {
    let mut cache = HashMap::new();
    let mut result = Vec::new();
    for process in facts.processes {
        let mut chain = hints::chain(facts, process);
        let found = chain.iter().enumerate().find_map(|(index, evidence)| {
            let ancestor = facts.by_pid[&evidence.identity.pid];
            cache
                .entry(ancestor.pid)
                .or_insert_with(|| session(ancestor, reader))
                .clone()
                .map(|name| (index, name))
        });
        let Some((index, name)) = found else { continue };
        chain.truncate(index + 1);
        let basis = if index == 0 {
            "arguments"
        } else {
            "ancestor_arguments"
        };
        result.extend(hints::proposal(
            "playwright",
            hints::value(
                "role",
                if index == 0 {
                    "Playwright CLI daemon"
                } else {
                    "Playwright-launched process"
                },
            ),
            basis,
            None,
            chain.clone(),
        ));
        result.extend(hints::proposal(
            "playwright",
            hints::value("session", &name),
            basis,
            None,
            chain.clone(),
        ));
        if project_enabled {
            let ancestor = facts.by_pid[&chain.last().expect("nonempty chain").identity.pid];
            if let Some(project) = ancestor
                .cwd
                .as_deref()
                .and_then(|cwd| projects::find(facts, cwd))
            {
                let name = project
                    .root
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy();
                result.extend(hints::proposal(
                    "playwright",
                    hints::value("project", &name),
                    if index == 0 { "cwd" } else { "ancestor_cwd" },
                    Some(project.root),
                    chain,
                ));
            }
        }
    }
    result
}

fn execute(context: &Context<'_>, request: Request<'_>) -> anyhow::Result<Contribution> {
    let Request::Classify(facts) = request else {
        anyhow::bail!("unsupported extension request")
    };
    let mut reader = Shared {
        provider: "playwright",
        reader: &context.argv,
    };
    Ok(Contribution::Classify {
        proposals: proposals(facts, &mut reader, !context.settings.is_disabled("project")),
    })
}

#[cfg(test)]
mod tests;

fn uuid_only(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| {
            if [8, 13, 18, 23].contains(&index) {
                byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}

fn summary(context: &crate::extensions::SummaryContext<'_>) -> Option<crate::extensions::Summary> {
    let session = context.hints().find(|hint| {
        hint.kind == "session"
            && matches!(hint.basis.as_str(), "arguments" | "ancestor_arguments")
            && !uuid_only(&hint.value)
    });
    if let Some(hint) = session {
        let mut summary = crate::extensions::Summary::new(
            "Playwright session",
            &hint.value,
            10,
            vec![hint.clone()],
        );
        summary.inherited = hint.basis == "ancestor_arguments";
        return Some(summary);
    }
    if let Some(summary) = super::project::summary(context) {
        return Some(summary);
    }
    let hint = context.hints().find(|hint| {
        hint.kind == "role" && matches!(hint.basis.as_str(), "arguments" | "ancestor_arguments")
    })?;
    let mut summary =
        crate::extensions::Summary::new("Tool role", &hint.value, 30, vec![hint.clone()]);
    summary.inherited = hint.basis == "ancestor_arguments";
    Some(summary)
}
