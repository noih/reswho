use crate::extensions::{
    Capability, Context, Contribution, Request,
    host::{
        Facts, Module, hints, no_registration, no_settings,
        runtime::{Argv, Shared},
    },
};

mod sessions;

pub(crate) static MODULE: Module = Module {
    summary: Some(summary),
    registration: no_registration,
    decorate: None,
    id: "codex",
    capabilities: &[Capability::Classify],
    execute,
    settings: no_settings,
};

fn execute(context: &Context<'_>, request: Request<'_>) -> anyhow::Result<Contribution> {
    let Request::Classify(facts) = request else {
        anyhow::bail!("unsupported extension request")
    };
    let mut reader = Shared {
        provider: "codex",
        reader: &context.argv,
    };
    Ok(Contribution::Classify {
        proposals: proposals(facts, &mut reader),
    })
}

fn proposals(
    facts: &Facts<'_>,
    reader: &mut impl Argv,
) -> Vec<crate::extensions::ClassificationProposal> {
    let mut result = Vec::new();
    for process in facts.processes {
        if process.uid != Some(unsafe { libc::geteuid() }) {
            continue;
        }
        let Some(executable) = &process.executable else {
            continue;
        };
        if executable.file_name().is_none_or(|name| name != "codex") {
            continue;
        }
        let Some(argv) = reader.read(process) else {
            continue;
        };
        let role = match argv.get(1).map(String::as_str) {
            Some("app-server") if argv.iter().any(|arg| arg == "--managed-daemon") => {
                "Codex managed daemon"
            }
            Some("app-server") => "Codex app server",
            Some("exec" | "e") => "Codex exec",
            _ => "Codex CLI",
        };
        let chain = hints::chain(facts, process).into_iter().take(1).collect();
        result.extend(hints::proposal(
            "codex",
            hints::value("role", role),
            "arguments",
            None,
            chain,
        ));
        result.extend(session_proposals(facts, process));
        let chain = hints::chain(facts, process).into_iter().take(1).collect();
        result.extend(hints::proposal("codex", hints::value("session_unavailable",
            "Current session unsupported: launch arguments and title indexes do not establish a live PID-to-session mapping"),
            "arguments", None, chain));
    }
    result
}

fn session_proposals(
    facts: &Facts<'_>,
    process: &crate::model::Process,
) -> Vec<crate::extensions::ClassificationProposal> {
    sessions::observe(process)
        .into_iter()
        .filter_map(|hint| {
            let chain = hints::chain(facts, process).into_iter().take(1).collect();
            hints::proposal("codex", hint, "open_session_log", None, chain)
        })
        .collect()
}

#[cfg(test)]
mod tests;

fn summary(context: &crate::extensions::SummaryContext<'_>) -> Option<crate::extensions::Summary> {
    use crate::extensions::Summary;
    let logs: Vec<_> = context
        .hints()
        .filter(|hint| hint.kind == "session_log" && hint.basis == "open_session_log")
        .cloned()
        .collect();
    let role = context
        .hints()
        .find(|hint| hint.kind == "role" && hint.basis == "arguments");
    if logs.len() > 1 {
        return Some(Summary::new(
            role.map_or("Codex", |hint| hint.value.as_str()),
            &format!("{} session logs", logs.len()),
            0,
            logs,
        ));
    }
    if let Some(log) = logs.first()
        && let Some(title) = log
            .title
            .as_deref()
            .filter(|title| !title.trim().is_empty())
    {
        return Some(Summary::new("Codex log", title, 0, logs.clone()));
    }
    role.map(|hint| Summary::new("Tool role", &hint.value, 30, vec![hint.clone()]))
}
