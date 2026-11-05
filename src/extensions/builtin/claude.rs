use crate::extensions::{
    Capability, Context, Contribution, Request,
    host::{
        Module, hints, no_registration, no_settings,
        runtime::{Argv, Shared, resolve},
    },
};

pub(crate) static MODULE: Module = Module {
    summary: Some(summary),
    registration: no_registration,
    decorate: None,
    id: "claude",
    capability: Capability::Classify,
    execute,
    settings: no_settings,
};

fn execute(context: &Context<'_>, request: Request<'_>) -> anyhow::Result<Contribution> {
    let Request::Classify(facts) = request else {
        anyhow::bail!("unsupported extension request")
    };
    let mut reader = Shared {
        provider: "claude",
        reader: &context.argv,
    };
    Ok(Contribution::Classify {
        proposals: proposals(facts, &mut reader),
    })
}

fn proposals(
    facts: &crate::extensions::host::Facts<'_>,
    reader: &mut impl Argv,
) -> Vec<crate::extensions::ClassificationProposal> {
    let mut proposals = Vec::new();
    for process in facts.processes {
        if process.uid != Some(unsafe { libc::geteuid() }) {
            continue;
        }
        let Some(executable) = &process.executable else {
            continue;
        };
        let native = executable.file_name().is_some_and(|name| name == "claude")
            || executable
                .parent()
                .is_some_and(|path| path.ends_with("claude/versions"))
                && executable
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(super::super::host::files::version_text);
        let node = executable
            .file_name()
            .is_some_and(|name| name == "node" || name == "nodejs");
        if !native && !node {
            continue;
        }
        let Some(argv) = reader.read(process).filter(|argv| !argv.is_empty()) else {
            continue;
        };
        let arguments = if native {
            &argv[1..]
        } else {
            let Some(entry) = super::node::entry(argv.clone(), executable, process.cwd.as_deref())
            else {
                continue;
            };
            if !entry.ends_with("@anthropic-ai/claude-code/cli.js") {
                continue;
            }
            let crate::extensions::host::files::Trusted::Bytes(bytes) =
                crate::extensions::host::files::read(
                    &entry.parent().expect("absolute entry").join("package.json"),
                    process.uid,
                )
            else {
                continue;
            };
            let Ok(package) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                continue;
            };
            if package["name"] != "@anthropic-ai/claude-code"
                || !package["version"]
                    .as_str()
                    .is_some_and(crate::extensions::host::files::version_text)
            {
                continue;
            }
            let Some(index) = argv
                .iter()
                .position(|arg| resolve(arg, process.cwd.as_deref()).as_ref() == Some(&entry))
            else {
                continue;
            };
            &argv[index + 1..]
        };
        let role = match arguments.first().map(String::as_str) {
            Some("--daemon") => "Claude Code daemon",
            Some("--bg-worker" | "--bg-spare") => "Claude Code background worker",
            _ => "Claude Code CLI",
        };
        for hint in [
            hints::value("role", role),
            hints::value(
                "session_unavailable",
                "Current session unsupported: PID registry updates can fail and process-start tokens have second precision",
            ),
        ] {
            let chain = hints::chain(facts, process).into_iter().take(1).collect();
            proposals.extend(hints::proposal("claude", hint, "arguments", None, chain));
        }
    }
    proposals
}

#[cfg(test)]
mod tests;

fn summary(context: &crate::extensions::SummaryContext<'_>) -> Option<crate::extensions::Summary> {
    let hint = context
        .hints()
        .find(|hint| hint.kind == "role" && hint.basis == "arguments")?;
    Some(crate::extensions::Summary::new(
        "Tool role",
        &hint.value,
        30,
        vec![hint.clone()],
    ))
}
