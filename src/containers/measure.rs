use super::{
    build,
    command::{Output, Runner, RuntimeCommand},
    docker,
};
use crate::snapshot::Cancellation;
use anyhow::Result;
use serde::Serialize;
use std::time::{Duration, Instant};

#[derive(Serialize)]
struct CommandTiming {
    phase: &'static str,
    elapsed_ms: f64,
    containers: usize,
    output_bytes: usize,
    diagnostic: bool,
}

struct TimedRunner {
    inner: RuntimeCommand,
    commands: Vec<CommandTiming>,
    stats_started: bool,
}

impl Runner for TimedRunner {
    fn limits(&self) -> Option<(Cancellation, Instant)> {
        self.inner.limits()
    }

    fn direct_stats(&mut self, host: &str, ids: &[String]) -> Result<Option<super::stats::Batch>> {
        let start = Instant::now();
        let result = self.inner.direct_stats(host, ids);
        if !matches!(result, Ok(None)) {
            self.stats_started = true;
            self.commands.push(CommandTiming {
                phase: "stats.api",
                elapsed_ms: start.elapsed().as_secs_f64() * 1000.0,
                containers: ids.len(),
                output_bytes: 0,
                diagnostic: !result.as_ref().is_ok_and(|batch| {
                    batch
                        .as_ref()
                        .is_some_and(|batch| batch.unavailable.is_empty())
                }),
            });
        }
        result
    }

    fn run(&mut self, args: &[String]) -> Result<Output> {
        self.run_until(args, self.inner.deadline)
    }

    fn run_until(&mut self, args: &[String], deadline: Instant) -> Result<Output> {
        let command: Vec<_> = args
            .iter()
            .map(String::as_str)
            .skip_while(|arg| arg.starts_with("--context=") || arg.starts_with("--host="))
            .collect();
        let phase = match command.as_slice() {
            ["context", "show", ..] => "context.show",
            ["context", "inspect", ..] => "context.inspect",
            ["info", ..] => "info",
            ["container", "ls", ..] => "inventory",
            ["container", "inspect", ..] if self.stats_started => "inspect.after",
            ["container", "inspect", ..] => "inspect.before",
            ["container", "stats", ..] => {
                self.stats_started = true;
                "stats"
            }
            ["--version"] => "cli.version",
            _ => "other",
        };
        let containers = if phase.starts_with("inspect.") || phase == "stats" {
            args.iter()
                .position(|arg| arg == "--")
                .map_or(0, |index| args.len() - index - 1)
        } else {
            0
        };
        let start = Instant::now();
        let result = self.inner.run_until(args, deadline);
        self.commands.push(CommandTiming {
            phase,
            elapsed_ms: start.elapsed().as_secs_f64() * 1000.0,
            containers,
            output_bytes: result.as_ref().map_or(0, |output| output.stdout.len()),
            diagnostic: !result.as_ref().is_ok_and(|output| output.issue.is_none()),
        });
        result
    }
}

#[test]
#[ignore = "read-only live Docker timing; three rounds, no service lifecycle changes"]
fn measure_docker_collection() {
    let deadline = Instant::now() + Duration::from_secs(40);
    for round in 1..=3 {
        assert!(Instant::now() < deadline, "measurement budget exhausted");
        let mut runner = TimedRunner {
            inner: RuntimeCommand {
                cancel: Cancellation::default(),
                deadline: deadline.min(Instant::now() + Duration::from_secs(2)),
                program: "docker".into(),
                sample_interval: Duration::from_millis(250),
                api_version: std::env::var("DOCKER_API_VERSION").ok(),
            },
            commands: Vec::new(),
            stats_started: false,
        };
        let version = runner.run(&["--version".into()]);
        assert!(
            version.is_ok_and(|output| output.issue.is_none()),
            "Docker CLI baseline unavailable"
        );
        runner.inner.deadline = deadline.min(Instant::now() + Duration::from_secs(10));
        let start = Instant::now();
        let (runtime, containers) = docker::collect(
            &mut runner,
            std::env::var("DOCKER_CONTEXT").ok(),
            std::env::var("DOCKER_HOST").ok(),
            time::OffsetDateTime::now_utc(),
            &[],
        );
        let collection_ms = start.elapsed().as_secs_f64() * 1000.0;
        let command_ms: f64 = runner
            .commands
            .iter()
            .filter(|item| item.phase != "cli.version")
            .map(|item| item.elapsed_ms)
            .sum();
        let diagnostics: Vec<_> = runtime
            .unavailable
            .iter()
            .chain(
                containers
                    .iter()
                    .flat_map(|container| &container.unavailable),
            )
            .map(|issue| (&issue.code, &issue.operation))
            .collect();
        let complete =
            runtime.status == "available" && runtime.inventory_complete && diagnostics.is_empty();
        println!(
            "{}",
            serde_json::json!({
                "round": round, "collection_ms": collection_ms, "non_command_ms": collection_ms - command_ms,
                "containers": containers.len(), "active": containers.iter().filter(|container| container.is_active()).count(),
                "complete": complete, "diagnostics": diagnostics, "commands": runner.commands,
            })
        );
        let start = Instant::now();
        let tree = build(runtime, containers).expect("container tree");
        println!(
            "{}",
            serde_json::json!({"round": round, "tree_ms": start.elapsed().as_secs_f64() * 1000.0, "runtime_groups": tree.children.len()})
        );
        assert!(
            complete,
            "incomplete collection; timings above include diagnostic codes"
        );
    }
}

#[test]
#[ignore = "read-only full snapshot timing; three rounds, no service lifecycle changes"]
fn measure_full_snapshot() {
    let deadline = Instant::now() + Duration::from_secs(40);
    for round in 1..=3 {
        assert!(Instant::now() < deadline, "measurement budget exhausted");
        let snapshot = crate::snapshot::capture(
            &crate::snapshot::Options {
                settings: std::sync::Arc::new(crate::extensions::default_settings().unwrap()),
                containers: true,
                command_pids: Vec::new(),
                detail_refs: Vec::new(),
            },
            &Cancellation::default(),
        )
        .expect("snapshot collection");
        let root =
            crate::tree::select(&snapshot.root, "group:containers").expect("container scope");
        let totals = root.container_totals.as_ref().expect("container totals");
        let runtimes: Vec<_> = root
            .children
            .iter()
            .filter_map(|node| node.runtime.as_ref())
            .collect();
        let mut pending = vec![&root];
        let mut bases = std::collections::BTreeSet::new();
        let mut container_diagnostics = 0;
        while let Some(node) = pending.pop() {
            if let Some(container) = &node.container {
                if container.is_active() {
                    bases.insert(&container.memory_basis);
                }
                container_diagnostics += container.unavailable.len();
            }
            pending.extend(&node.children);
        }
        let complete = runtimes
            .iter()
            .all(|runtime| runtime.inventory_complete && runtime.unavailable.is_empty())
            && container_diagnostics == 0
            && totals.cpu_unknown == 0
            && totals.memory_unknown == 0;
        println!(
            "{}",
            serde_json::json!({
                "round": round, "collection_ms": snapshot.collection_ms, "host_sample_ms": snapshot.sample_ms,
                "host_processes": snapshot.root.totals.processes, "runtimes":runtimes.iter().map(|r| (&r.name, r.collection_ms)).collect::<Vec<_>>(),
                "containers":totals.containers, "active":totals.active_containers,
                "cpu_unknown":totals.cpu_unknown, "memory_unknown":totals.memory_unknown,
                "memory_bases":bases, "complete":complete, "container_diagnostics":container_diagnostics,
                "runtime_diagnostics":runtimes.iter().flat_map(|r| &r.unavailable).map(|d| (&d.code, &d.operation)).collect::<Vec<_>>(),
                "host_warning_count":snapshot.warnings.len(),
            })
        );
        assert!(complete, "incomplete collection; see anonymous summary");
    }
}
