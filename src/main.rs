use anyhow::{Context, Result};
use clap::{ArgGroup, Parser};
use reswho::{
    output,
    snapshot::{self, Cancellation, Options},
    tree, tui,
};
use std::{
    io::{self, IsTerminal, Write},
    time::Duration,
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
    #[arg(
        long,
        help = "Open the interactive resource tree (requires a terminal)"
    )]
    tui: bool,
    #[arg(long, help = "Explain output fields, units and missing values")]
    help_format: bool,
    #[arg(long, default_value = "group:all", value_parser = parse_reference, help = "Select a pid:<number> or group:<id> branch")]
    from: String,
    #[arg(long, default_value = "2", value_parser = parse_depth, help = "Output depth / initial TUI expansion (0..128 or all)")]
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
            return tui::run(
                Options {
                    sample_interval: Duration::from_millis(args.sample_ms),
                    containers: !args.no_containers,
                },
                args.from,
                args.depth,
            );
        }
        let mut snapshot = snapshot::capture(
            &Options {
                sample_interval: Duration::from_millis(args.sample_ms),
                containers: !args.no_containers,
            },
            &Cancellation::default(),
        )?;
        snapshot.root = tree::select(&snapshot.root, &args.from)?;
        snapshot.scope = args.from;
        if args.json {
            tree::limit_depth(&mut snapshot.root, args.depth);
            format!("{}\n", serde_json::to_string_pretty(&snapshot)?)
        } else {
            output::markdown_with_depth(&snapshot, args.depth)
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
