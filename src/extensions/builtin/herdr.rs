use crate::{
    extensions::{
        Capability, Context, Contribution, ProcessIdentity, Request, Summary, SummaryContext,
        host::{
            Facts, Module, hints, no_settings,
            runtime::{Argv, PREFIX_ARGS, Shared},
        },
        placement::{Category, Ownership, Registration},
    },
    model::{DetailField, Process},
};

pub(crate) static MODULE: Module = Module {
    summary: Some(summary),
    registration,
    decorate: None,
    id: "herdr",
    capabilities: &[Capability::Classify, Capability::Details],
    execute,
    settings: no_settings,
};

fn registration() -> Registration {
    Registration {
        categories: vec![Category::new(
            "extensions.herdr.services",
            "Shared services",
            150,
        )],
        ownership: vec![Ownership::new(
            "extensions.herdr.role",
            "extensions.herdr.services",
            75,
        )],
    }
}

fn candidate(process: &Process) -> bool {
    process.uid == Some(unsafe { libc::geteuid() })
        && process.executable.as_ref().is_some_and(|path| {
            path.is_absolute() && path.file_name().is_some_and(|name| name == "herdr")
        })
}

fn server(argv: &[String]) -> bool {
    if argv.is_empty() || argv.len() >= PREFIX_ARGS {
        return false;
    }
    let valid_session = |name: &str| {
        !name.is_empty()
            && name.len() <= 64
            && !matches!(name, "." | "..")
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    };
    let mut args = Vec::new();
    let mut rest = argv[1..].iter();
    while let Some(arg) = rest.next() {
        if arg == "--session" {
            if rest.next().is_none_or(|name| !valid_session(name)) {
                return false;
            }
        } else if let Some(name) = arg.strip_prefix("--session=") {
            if !valid_session(name) {
                return false;
            }
        } else {
            args.push(arg.as_str());
        }
    }
    matches!(
        args.as_slice(),
        ["server"] | ["server", "--handoff-import", _, _]
    )
}

fn execute(context: &Context<'_>, request: Request<'_>) -> anyhow::Result<Contribution> {
    let mut reader = Shared {
        provider: "herdr",
        reader: &context.argv,
    };
    match request {
        Request::Classify(facts) => Ok(Contribution::Classify {
            proposals: proposals(facts, &mut reader),
        }),
        Request::Details { process, .. } => Ok(Contribution::ParsedDetails {
            identity: ProcessIdentity::observed(process)
                .ok_or_else(|| anyhow::anyhow!("Process identity unavailable"))?,
            fields: details(process, &mut reader),
        }),
        _ => anyhow::bail!("unsupported extension request"),
    }
}

fn proposals(
    facts: &Facts<'_>,
    reader: &mut impl Argv,
) -> Vec<crate::extensions::ClassificationProposal> {
    facts
        .processes
        .iter()
        .filter(|process| candidate(process))
        .filter_map(|process| {
            let identity = ProcessIdentity::observed(process)?;
            if !server(&reader.read(process)?) {
                return None;
            }
            hints::proposal(
                "herdr",
                hints::value("role", "Herdr session server"),
                "arguments",
                None,
                vec![crate::extensions::tags::ProcessEvidence {
                    identity,
                    executable: process.executable.clone()?,
                }],
            )
        })
        .collect()
}

fn details(process: &Process, reader: &mut impl Argv) -> Vec<DetailField> {
    if !candidate(process) {
        return vec![];
    }
    let role = reader.read(process).map(|args| server(&args));
    let fields = match role {
        Some(true) => vec![
            (
                "scope",
                "Service scope",
                "Session-wide: can host workspaces from multiple projects; cwd is a launch association",
            ),
            (
                "evidence",
                "Scope evidence",
                "Verified process identity, Herdr executable and server launch mode",
            ),
            (
                "clients",
                "Current clients",
                "Unknown: launch mode does not establish active clients or their projects",
            ),
        ],
        Some(false) => vec![(
            "scope",
            "Service scope",
            "Unknown: no supported server launch mode observed",
        )],
        None => vec![(
            "scope",
            "Service scope",
            "Unknown: launch arguments unavailable or identity changed",
        )],
    };
    fields
        .into_iter()
        .map(|(key, label, value)| DetailField {
            key: format!("extensions.herdr.{key}"),
            label: label.into(),
            value: value.into(),
            sensitive: false,
        })
        .collect()
}

fn summary(context: &SummaryContext<'_>) -> Option<Summary> {
    let role = context
        .hints()
        .find(|hint| hint.kind == "role" && hint.basis == "arguments")?;
    Some(Summary::new(
        "Shared scope",
        "Herdr session server",
        0,
        vec![role.clone()],
    ))
}

#[cfg(test)]
mod tests;
