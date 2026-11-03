use anyhow::{Context, Result};
use clap::{ArgGroup, Parser, Subcommand};
use reswho::{
    extensions, output, settings,
    snapshot::{self, Cancellation, Options},
    sort::Sort,
    tree, tui,
};
use std::{
    io::{self, IsTerminal, Write},
    path::PathBuf,
    sync::Arc,
};

#[derive(Debug, Parser)]
#[command(
    version,
    about = "Read-only macOS resource attribution: processes, containers and resources",
    after_help = "With no output flag, terminals open the TUI; pipes emit Markdown.
--md, --json and --tui are mutually exclusive. --json-schema also conflicts
with these formats and --help-md.

Examples:
  reswho --md --sort cpu                Inspect a snapshot by CPU usage
  reswho --md --from pid:1234           Read one PID's details
  reswho --md --from group:containers --depth all
  reswho --help-md                      Read the Markdown output guide
  reswho settings --json                Inspect effective settings offline
  reswho --set sampling.interval_ms=500 --md
  reswho settings --set extensions.example.label=work

--set accepts registered core, shared.* and extensions.<id>.* keys.
Extension keys require available manifest definitions; strings are raw text."
)]
#[command(group(ArgGroup::new("format").args(["md", "json", "tui"])))]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,
    #[arg(
        long,
        help_heading = "Output",
        help = "Emit a single Markdown snapshot"
    )]
    md: bool,
    #[arg(
        long,
        help_heading = "Output",
        help = "Append the extension timing table to Markdown output"
    )]
    profile: bool,
    #[arg(
        long,
        global = true,
        help_heading = "Output",
        help = "Emit JSON (snapshot or offline settings/extensions)"
    )]
    json: bool,
    #[arg(long, help_heading = "Output", conflicts_with_all = ["md", "json", "tui", "help_md"], help = "Print the JSON Schema without collecting resources")]
    json_schema: bool,
    #[arg(
        long,
        help_heading = "Output",
        help = "Open the interactive resource tree (requires a terminal)"
    )]
    tui: bool,
    #[arg(
        long = "help-md",
        help_heading = "Output",
        help = "Explain Markdown output offline: rows, units and symbols"
    )]
    help_md: bool,
    #[arg(long, help_heading = "Selection/expansion", default_value = "group:all", value_parser = parse_reference, help = "Select pid:<number> or group:<id>")]
    from: String,
    #[arg(long, help_heading = "Selection/expansion", next_line_help = true, default_value = "2", value_parser = parse_depth, help = "Depth / initial TUI expansion (0..128 or all)")]
    depth: usize,
    #[arg(
        long,
        global = true,
        help_heading = "Selection/expansion",
        help = "Skip container runtime queries"
    )]
    no_containers: bool,
    #[arg(
        long,
        help_heading = "Sorting",
        value_enum,
        default_value = "memory",
        hide_possible_values = true,
        next_line_help = true,
        help = "Sibling order: cpu/memory/uptime descending; name natural ascending.
Sets initial TUI sort"
    )]
    sort: Sort,
    #[arg(
        long,
        global = true,
        value_name = "KEY=VALUE",
        next_line_help = true,
        help_heading = "Configuration",
        help = "Override a registered setting; repeatable, last value wins"
    )]
    set: Vec<String>,
    #[arg(
        long,
        global = true,
        help_heading = "Configuration",
        help = "Read JSON settings from PATH"
    )]
    config: Option<PathBuf>,
}

impl Args {
    fn validate_formats(&self) -> Result<(), clap::Error> {
        if u8::from(self.md) + u8::from(self.json) + u8::from(self.tui) > 1
            || (self.json_schema && (self.md || self.json || self.tui || self.help_md))
        {
            return Err(clap::Error::raw(
                clap::error::ErrorKind::ArgumentConflict,
                "--md, --json and --tui are mutually exclusive; --json-schema also conflicts with these formats and --help-md",
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Subcommand)]
enum Command {
    #[command(about = "Show effective settings offline without collecting resources")]
    Settings,
    #[command(about = "List built-in/installed extensions offline; manage sources")]
    Extensions {
        #[command(subcommand)]
        action: Option<ExtensionAction>,
    },
}

#[derive(Debug, Subcommand)]
enum ExtensionAction {
    #[command(about = "Register an external extension from its manifest")]
    Install { manifest: PathBuf },
    #[command(about = "Enable an installed external extension")]
    Enable { id: String },
    #[command(about = "Disable an installed external extension")]
    Disable { id: String },
    #[command(about = "Unregister an external extension without deleting its files")]
    Remove { id: String },
}

fn parse_reference(value: &str) -> Result<String, String> {
    let valid = value
        .strip_prefix("pid:")
        .is_some_and(|pid| pid.parse::<u32>().is_ok_and(|pid| pid > 0))
        || value.strip_prefix("group:").is_some_and(|group| {
            !group.is_empty() && group.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        });
    if valid {
        Ok(value.into())
    } else {
        Err("expected pid:<positive number> or group:<id>".into())
    }
}

fn parse_depth(value: &str) -> Result<usize, String> {
    if value == "all" {
        return Ok(usize::MAX);
    }
    value
        .parse::<usize>()
        .ok()
        .filter(|depth| *depth <= 128)
        .ok_or_else(|| "expected 0..128 or all".into())
}

fn run() -> Result<()> {
    let arguments: Vec<_> = std::env::args_os().collect();
    let mut args = Args::parse_from(&arguments);
    args.set.clear();
    let mut arguments = arguments.iter().skip(1);
    while let Some(argument) = arguments.next() {
        if argument == "--" {
            break;
        }
        if argument == "--set" {
            args.set.push(
                arguments
                    .next()
                    .expect("Clap checked value")
                    .to_str()
                    .expect("Clap checked UTF-8")
                    .to_owned(),
            );
        } else if let Some(value) = argument.to_str().and_then(|arg| arg.strip_prefix("--set=")) {
            args.set.push(value.to_owned());
        }
    }
    if let Err(error) = args.validate_formats() {
        error.exit();
    }
    let content = if args.json_schema {
        output::JSON_SCHEMA.into()
    } else if args.help_md {
        output::FORMAT_HELP.into()
    } else if let Some(Command::Extensions {
        action: Some(action),
    }) = &args.command
    {
        anyhow::ensure!(
            args.set.is_empty(),
            "--set cannot be used with extension lifecycle actions"
        );
        let path = extensions::state::State::path(args.config.as_deref())?;
        extensions::state::State::startup(&path)?;
        extensions::state::State::mutate(&path, |state| match action {
            ExtensionAction::Install { manifest } => {
                state.install_explicit(extensions::state::Manifest::read(manifest)?, manifest)
            }
            ExtensionAction::Enable { id } => state.toggle(id, true, false),
            ExtensionAction::Disable { id } => state.toggle(id, false, false),
            ExtensionAction::Remove { id } => state.toggle(id, false, true),
        })?;
        "Extension registry updated\n".into()
    } else {
        let mut state = extensions::state::State::startup(&extensions::state::State::path(
            args.config.as_deref(),
        )?)?;
        let user = settings::Config::load(args.config.as_deref())?;
        let registry = state.registry_reported()?;
        let overrides = registry.parse_overrides(&args.set)?;
        let resolved = registry.resolve(&user, &overrides)?;
        for key in &resolved.report().unknown_keys {
            eprintln!("reswho: unknown setting {key} (not applied)");
        }
        if matches!(args.command, Some(Command::Extensions { .. })) {
            let entries = state.list(&resolved, !args.no_containers);
            if args.json {
                format!("{}\n", serde_json::to_string_pretty(&entries)?)
            } else {
                entries
                    .iter()
                    .map(|entry| {
                        format!(
                            "{}/{} source={} capabilities={} enabled={} availability={} requires={} compatible={} reason={}\n",
                            entry.descriptor.id,
                            entry.descriptor.version,
                            entry.descriptor.source,
                            entry
                                .descriptor
                                .capabilities
                                .iter()
                                .map(|capability| format!("{capability:?}").to_lowercase())
                                .collect::<Vec<_>>()
                                .join(","),
                            entry.descriptor.enabled,
                            entry.descriptor.availability,
                            entry.requires, entry.compatible, entry.reason.as_deref().unwrap_or("none")
                        )
                    })
                    .collect()
            }
        } else if matches!(args.command, Some(Command::Settings)) {
            let report = resolved.report();
            if args.json {
                format!("{}\n", serde_json::to_string_pretty(report)?)
            } else {
                report
                    .settings
                    .iter()
                    .map(|(key, setting)| {
                        format!(
                            "{key} = {} (source: {}, definition: {}/{})\n",
                            setting.value,
                            setting.source,
                            setting.definition.provider,
                            setting.definition.version
                        )
                    })
                    .collect()
            }
        } else {
            let refresh_interval = resolved.interval("refresh.interval_ms");
            let resolved = Arc::new(resolved);
            if args.tui
                || (!args.md
                    && !args.json
                    && io::stdin().is_terminal()
                    && io::stdout().is_terminal())
            {
                return tui::run(
                    Options {
                        settings: Arc::clone(&resolved),
                        extensions: Arc::new(state.clone()),
                        containers: !args.no_containers,
                        command_pid: None,
                    },
                    args.from,
                    args.depth,
                    args.sort,
                    refresh_interval,
                    settings::edit::Settings::new(
                        registry,
                        user,
                        overrides,
                        args.config.or_else(settings::default_path),
                    ),
                );
            }
            let mut snapshot = snapshot::capture(
                &Options {
                    settings: Arc::clone(&resolved),
                    extensions: Arc::new(state.clone()),
                    containers: !args.no_containers,
                    command_pid: args
                        .from
                        .strip_prefix("pid:")
                        .and_then(|pid| pid.parse().ok()),
                },
                &Cancellation::default(),
            )?;
            snapshot.root = tree::select(&snapshot.root, &args.from)?;
            snapshot.scope = args.from;
            args.sort.apply(&mut snapshot.root);
            if args.json {
                tree::limit_depth(&mut snapshot.root, args.depth);
                format!("{}\n", serde_json::to_string_pretty(&snapshot)?)
            } else {
                let mut text = output::markdown_with_depth(&snapshot, args.depth);
                if args.profile {
                    text.push_str(&output::extension_profile(&snapshot.extensions));
                }
                text
            }
        }
    };
    match io::stdout().lock().write_all(content.as_bytes()) {
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        result => result.context("write output"),
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("reswho: {error:#}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_json_conflicts_are_checked_after_subcommand_propagation() {
        for flag in ["--json-schema", "--md", "--tui"] {
            for arguments in [
                vec!["reswho", flag, "settings", "--json"],
                vec!["reswho", flag, "--json", "settings"],
                vec!["reswho", "--json", flag, "settings"],
                vec!["reswho", "--json", "settings", flag],
                vec!["reswho", "settings", flag, "--json"],
                vec!["reswho", "settings", "--json", flag],
            ] {
                assert!(
                    Args::try_parse_from(&arguments)
                        .and_then(|args| args.validate_formats())
                        .is_err(),
                    "{arguments:?}"
                );
            }
        }
        for arguments in [
            ["reswho", "settings", "--json"],
            ["reswho", "--json", "settings"],
        ] {
            Args::try_parse_from(arguments)
                .unwrap()
                .validate_formats()
                .unwrap();
        }
    }

    #[test]
    fn modes_conflict_and_selectors_are_validated() {
        assert!(Args::try_parse_from(["reswho", "--json", "--md"]).is_err());
        assert!(Args::try_parse_from(["reswho", "--from", "pid:0"]).is_err());
        assert!(Args::try_parse_from(["reswho", "--depth", "-1"]).is_err());
        assert!(Args::try_parse_from(["reswho", "--depth", "129"]).is_err());
        assert!(Args::try_parse_from(["reswho", "--sort", "random"]).is_err());
        assert_eq!(Args::try_parse_from(["reswho"]).unwrap().sort, Sort::Memory);
        for (value, expected) in [
            ("cpu", Sort::Cpu),
            ("memory", Sort::Memory),
            ("uptime", Sort::Uptime),
            ("name", Sort::Name),
        ] {
            for mode in ["--md", "--json", "--tui"] {
                assert_eq!(
                    Args::try_parse_from(["reswho", mode, "--sort", value])
                        .unwrap()
                        .sort,
                    expected
                );
            }
        }
        assert_eq!(
            Args::try_parse_from(["reswho", "--depth", "all"])
                .unwrap()
                .depth,
            usize::MAX
        );
    }
}
