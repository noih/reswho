use anyhow::{Context, Result, bail};
use clap::{ArgGroup, Parser};
use reswho::{collect, containers, model::Snapshot, output, tree};
use std::{
    io::{self, IsTerminal, Write},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Parser)]
#[command(
    version,
    about = "Read-only macOS resource attribution (collection prototype)"
)]
#[command(group(ArgGroup::new("format").args(["md", "json", "tui"])))]
struct Args {
    #[arg(long, help = "Emit a single Markdown snapshot")]
    md: bool,
    #[arg(long, help = "Emit a single JSON snapshot")]
    json: bool,
    #[arg(long, help = "Request TUI (not yet available in this prototype)")]
    tui: bool,
    #[arg(long, help = "Explain output fields, units and missing values")]
    help_format: bool,
    #[arg(long, default_value = "group:all", value_parser = parse_reference, help = "Select a pid:<number> or group:<id> branch")]
    from: String,
    #[arg(long, default_value = "2", value_parser = parse_depth, help = "Maximum depth below selected root (0..128 or all)")]
    depth: usize,
    #[arg(long, default_value = "250", value_parser = clap::value_parser!(u64).range(200..=5000), help = "Minimum CPU sampling interval in milliseconds")]
    sample_ms: u64,
    #[arg(long, help = "Skip container runtime queries")]
    no_containers: bool,
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
    let args = Args::parse();
    let content = if args.help_format {
        output::FORMAT_HELP.into()
    } else {
        if args.tui
            || (!args.md && !args.json && io::stdin().is_terminal() && io::stdout().is_terminal())
        {
            bail!("TUI is not implemented in this collection prototype; use --md or --json");
        }
        let start = Instant::now();
        let mut collection = collect::collect(Duration::from_millis(args.sample_ms))?;
        let mut root = tree::build(collection.processes)?;
        if !args.no_containers {
            match containers::collect() {
                Ok(containers) => root.children.push(containers),
                Err(error) => collection.warnings.push(format!("Containers: {error}")),
            }
        }
        tree::validate_references(&root)?;
        let mut root = tree::select(&root, &args.from)?;
        tree::limit_depth(&mut root, args.depth);
        let snapshot = Snapshot {
            schema: "reswho/7",
            captured_unix_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .context("system clock before Unix epoch")?
                .as_millis() as u64,
            sample_ms: collection.sample_ms,
            collection_ms: start.elapsed().as_millis() as u64,
            system: collection.system,
            scope: args.from,
            warnings: collection.warnings,
            root,
        };
        if args.json {
            format!("{}\n", serde_json::to_string_pretty(&snapshot)?)
        } else {
            output::markdown(&snapshot)
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
    fn modes_conflict_and_selectors_are_validated() {
        assert!(Args::try_parse_from(["reswho", "--json", "--md"]).is_err());
        assert!(Args::try_parse_from(["reswho", "--from", "pid:0"]).is_err());
        assert!(Args::try_parse_from(["reswho", "--depth", "-1"]).is_err());
        assert!(Args::try_parse_from(["reswho", "--depth", "129"]).is_err());
        assert_eq!(
            Args::try_parse_from(["reswho", "--depth", "all"])
                .unwrap()
                .depth,
            usize::MAX
        );
    }
}
