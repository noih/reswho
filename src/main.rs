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
  reswho --json --from pid:1 --from pid:2 --from group:containers
                                        Several selections from one snapshot
  reswho --md --from group:containers --depth all
  reswho --help-md                      Read the Markdown output guide
  reswho settings --json                Inspect effective settings offline
  reswho --set sampling.interval_ms=500 --md
  reswho settings --set extensions.node.entry_argv=false

--set accepts registered core, shared.* and extensions.<id>.* keys.
Extension keys require compiled module definitions; strings are raw text."
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
    #[arg(long, help_heading = "Selection/expansion", value_parser = parse_reference, next_line_help = true, help = "Select pid:<number> or group:<id> (default group:all).
Repeatable for --md/--json: one snapshot, a section per selection")]
    from: Vec<String>,
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
    #[command(about = "List extensions offline; enable or disable sources")]
    Extensions {
        #[command(subcommand)]
        action: Option<ExtensionAction>,
    },
}

#[derive(Debug, Subcommand)]
enum ExtensionAction {
    #[command(about = "Enable an extension")]
    Enable { id: String },
    #[command(about = "Disable an extension")]
    Disable { id: String },
}

fn parse_reference(value: &str) -> Result<String, String> {
    if let Some(pid) = value
        .strip_prefix("pid:")
        .and_then(|pid| pid.parse::<u32>().ok())
        .filter(|pid| *pid > 0)
    {
        return Ok(format!("pid:{pid}"));
    }
    if value.strip_prefix("group:").is_some_and(|group| {
        !group.is_empty() && group.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    }) {
        Ok(value.into())
    } else {
        Err("expected pid:<positive number> or group:<id>".into())
    }
}

fn selectors(from: &[String]) -> Vec<String> {
    let mut selectors: Vec<String> = Vec::new();
    for selector in from {
        if !selectors.contains(selector) {
            selectors.push(selector.clone());
        }
    }
    if selectors.is_empty() {
        selectors.push("group:all".into());
    }
    selectors
}

fn select(
    root: &reswho::model::Node,
    selectors: &[String],
    sort: Sort,
) -> Vec<reswho::model::Selection> {
    let mut selections: Vec<_> = selectors
        .iter()
        .map(|selector| {
            let (root, diagnostics) = match tree::select(root, selector) {
                Ok(mut node) => {
                    sort.apply(&mut node);
                    (Some(node), Vec::new())
                }
                Err(error) => (
                    None,
                    vec![reswho::model::Diagnostic::new(
                        "not_found",
                        "selection",
                        format!("{error:#}"),
                    )],
                ),
            };
            reswho::model::Selection {
                selector: selector.clone(),
                root,
                nested_in: Vec::new(),
                diagnostics,
            }
        })
        .collect();
    for index in 0..selections.len() {
        let nested_in = selections
            .iter()
            .enumerate()
            .filter(|(other, outer)| {
                *other != index
                    && selections[index].root.is_some()
                    && outer.root.as_ref().is_some_and(|outer| {
                        tree::select(outer, &selections[index].selector).is_ok()
                    })
            })
            .map(|(_, outer)| outer.selector.clone())
            .collect();
        selections[index].nested_in = nested_in;
    }
    selections
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

fn run() -> Result<usize> {
    let mut partial = 0;
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
        let (ExtensionAction::Enable { id } | ExtensionAction::Disable { id }) = action;
        let config_path = args.config.clone().or_else(settings::default_path);
        let baseline = settings::Config::load_path(config_path.as_deref(), false)?;
        let mut editor = settings::edit::Settings::new(
            extensions::settings_registry()?,
            baseline,
            Default::default(),
            config_path,
        );
        editor.save_extensions(
            None,
            None,
            &std::collections::BTreeMap::from([(
                id.clone(),
                matches!(action, ExtensionAction::Enable { .. }),
            )]),
        )?;
        "Extension settings updated\n".into()
    } else {
        let user = settings::Config::load(args.config.as_deref())?;
        let registry = extensions::settings_registry()?;
        let overrides = registry.parse_overrides(&args.set)?;
        let resolved = registry.resolve(&user, &overrides)?;
        for key in &resolved.report().unknown_keys {
            eprintln!("reswho: unknown setting {key} (not applied)");
        }
        if matches!(args.command, Some(Command::Extensions { .. })) {
            let entries = extensions::inventory::list(&resolved, !args.no_containers);
            if args.json {
                format!("{}\n", serde_json::to_string_pretty(&entries)?)
            } else {
                entries
                    .iter()
                    .map(|entry| {
                        format!(
                            "{} capabilities={} enabled={} availability={} reason={}\n",
                            entry.descriptor.id,
                            entry
                                .descriptor
                                .capabilities
                                .iter()
                                .map(|capability| format!("{capability:?}").to_lowercase())
                                .collect::<Vec<_>>()
                                .join(","),
                            entry.descriptor.enabled,
                            entry.descriptor.availability,
                            entry.reason.as_deref().unwrap_or("none"),
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
                            "{key} = {} (source: {}, definition: {})\n",
                            setting.value, setting.source, setting.definition.provider
                        )
                    })
                    .collect()
            }
        } else {
            let refresh_interval = resolved.interval("refresh.interval_ms");
            let resolved = Arc::new(resolved);
            let selectors = selectors(&args.from);
            anyhow::ensure!(
                !args.tui || selectors.len() == 1,
                "--tui accepts one --from; use --md or --json for several selections"
            );
            if args.tui
                || (!args.md
                    && !args.json
                    && selectors.len() == 1
                    && io::stdin().is_terminal()
                    && io::stdout().is_terminal())
            {
                return tui::run(
                    Options {
                        settings: Arc::clone(&resolved),
                        containers: !args.no_containers,
                        command_pids: Vec::new(),
                        detail_refs: Vec::new(),
                    },
                    selectors[0].clone(),
                    args.depth,
                    args.sort,
                    refresh_interval,
                    settings::edit::Settings::new(
                        registry,
                        user,
                        overrides,
                        args.config.or_else(settings::default_path),
                    ),
                )
                .map(|_| 0);
            }
            let snapshot = snapshot::capture(
                &Options {
                    settings: Arc::clone(&resolved),
                    containers: !args.no_containers,
                    detail_refs: selectors.clone(),
                    command_pids: selectors
                        .iter()
                        .filter_map(|selector| reswho::model::selector_pid(selector))
                        .collect(),
                },
                &Cancellation::default(),
            )?;
            let mut selections = select(&snapshot.root, &selectors, args.sort);
            let missing: Vec<_> = selections
                .iter()
                .filter(|selection| selection.root.is_none())
                .flat_map(|selection| &selection.diagnostics)
                .map(|diagnostic| diagnostic.message.clone())
                .collect();
            anyhow::ensure!(missing.len() < selections.len(), "{}", missing.join("; "));
            partial = missing.len();
            if args.json {
                for root in selections
                    .iter_mut()
                    .filter_map(|selection| selection.root.as_mut())
                {
                    tree::limit_depth(root, args.depth);
                }
                let report = reswho::model::Report {
                    snapshot: &snapshot,
                    selections: &selections,
                };
                format!("{}\n", serde_json::to_string_pretty(&report)?)
            } else {
                let mut text = output::markdown_selections(&snapshot, &selections, args.depth);
                if args.profile {
                    text.push_str(&output::extension_profile(&snapshot.extensions));
                }
                text
            }
        }
    };
    match io::stdout().lock().write_all(content.as_bytes()) {
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => Ok(partial),
        result => result.context("write output").map(|_| partial),
    }
}

fn main() {
    match run() {
        Ok(0) => {}
        Ok(missing) => {
            eprintln!("reswho: {missing} selection(s) not found; see their diagnostics");
            std::process::exit(3);
        }
        Err(error) => {
            eprintln!("reswho: {error:#}");
            std::process::exit(1);
        }
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
    fn repeated_from_mixes_kinds_normalizes_and_deduplicates_in_order() {
        let args = Args::try_parse_from([
            "reswho",
            "--json",
            "--from",
            "pid:7",
            "--from",
            "group:abc-1",
            "--from",
            "pid:07",
            "--from",
            "pid:8",
        ])
        .unwrap();
        assert_eq!(
            selectors(&args.from),
            ["pid:7", "group:abc-1", "pid:8"].map(String::from)
        );
        assert_eq!(selectors(&[]), ["group:all".to_owned()]);
        assert_eq!(
            selectors(
                &Args::try_parse_from(["reswho", "--from", "pid:3"])
                    .unwrap()
                    .from
            ),
            ["pid:3".to_owned()]
        );
        for invalid in [
            "pid:0",
            "pid:x",
            "group:",
            "group:a_b",
            "proc:1",
            "pid:4294967296",
        ] {
            assert!(
                Args::try_parse_from(["reswho", "--from", "pid:1", "--from", invalid]).is_err(),
                "{invalid}"
            );
        }
    }

    #[test]
    fn selections_keep_order_report_missing_and_nested_scopes() {
        let process = |pid, ppid| reswho::model::Process {
            pid,
            ppid: Some(ppid),
            started_unix_us: Some(u64::from(pid)),
            name: format!("p{pid}"),
            executable: Some("/opt/tool/run".into()),
            resident_bytes: Some(10),
            ..Default::default()
        };
        let root = tree::build(vec![process(10, 1), process(11, 10)]).unwrap();
        let selected = select(
            &root,
            &["pid:11", "group:all", "pid:4294967295", "pid:10"].map(String::from),
            Sort::Name,
        );
        let summary: Vec<_> = selected
            .iter()
            .map(|selection| {
                (
                    selection.selector.as_str(),
                    selection.root.is_some(),
                    selection.nested_in.clone(),
                    selection
                        .diagnostics
                        .iter()
                        .map(|d| d.code.clone())
                        .collect::<Vec<_>>(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            [
                (
                    "pid:11",
                    true,
                    vec!["group:all".into(), "pid:10".into()],
                    vec![]
                ),
                ("group:all", true, vec![], vec![]),
                (
                    "pid:4294967295",
                    false,
                    vec![],
                    vec!["not_found".to_owned()]
                ),
                ("pid:10", true, vec!["group:all".into()], vec![]),
            ]
        );
        assert!(
            selected[2].diagnostics[0]
                .message
                .contains("pid:4294967295")
        );
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
