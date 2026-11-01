use crate::model::{Network, Node, Process, Protocol, Snapshot, Totals};
use std::collections::BTreeSet;
use std::fmt::Write;

pub const FORMAT_HELP: &str = "reswho-md/4 (collection prototype)\n\n\
Process: [name] · [pid ref] · [CPU] · [RSS] · [uptime] · [ports]\n\
Group: [name] · [group ref] · [CPU sum] · [RSS sum] · oldest [uptime] · [process count]\n\
Uptime is elapsed time since process start, including idle time.\n\
Group oldest is the longest known member uptime, not active use or the group's lifetime.\n\
oldest >=2h means at least 2h with unknown members; oldest ? means unavailable.\n\
CPU 100% means one logical core for processes/groups; host CPU is 0–100%.\n\
Memory uses binary units (KiB/MiB/GiB). Process rows show own RSS.\n\
Group rows sum known descendant values once; shared pages can overlap.\n\
>= marks a partial sum with unknown members; ? means unavailable, - means known absent.\n\
Ports: tcp:N = TCP listening port; udp:N = bound UDP port (not proof of a server).\n\
A trailing + ? means partial socket coverage. Details show local/remote endpoints and TCP state.\n\
Sockets count unique observed TCP/UDP sockets per process, not descriptors; no cross-process sum.\n\
Socket snapshots are non-atomic; no traffic, Unix sockets or inactivity history is reported.\n\
Nested groups represent attribution; nested processes represent observed parenthood.\n\
Process rows are summaries; --from pid:N shows details for the selected PID only.\n\
Unreadable rows combine processes with no readable details; they cannot be expanded.\n\
JSON retains individual processes and diagnostics, including unreadable entries.\n\
PPID is preserved even when a parent belongs to a different group.\n\
Attribution uses bundle structure, Git roots and system locations, not app/tool name lists.\n\
Evidence: app PATH = application bundle; git PATH = Git root found from cwd.\n\
Ancestor attribution is labelled inferred and requires matching UID/cwd and compatible start times.\n\
A direct app/project assignment wins; different or missing cwd prevents ancestor inference.\n\
Cwd/exe paths are hints, not proof that a process is no longer needed.\n\
--from REF selects a branch; --depth N counts edges from that branch (root=0).\n\
Depth limits presentation only. ▸ marks collapsed children.\n\
Expand: reswho --md --from REF --depth N (or --depth all). Copy REF from the row.\n\
Group refs are deterministic, not display positions. PID refs query the current PID.\n\
JSON schema reswho/2 includes raw byte counts, unknown counts and sample timing.\n\
Command arguments, environment, container contents and review rules are not collected.\n";

pub fn markdown(snapshot: &Snapshot) -> String {
    let mut text = String::new();
    let _ = writeln!(
        text,
        "# reswho\nFormat: reswho-md/4 · help: reswho --help-format"
    );
    let _ = writeln!(
        text,
        "Captured: {}ms Unix · sample {}ms · collection {}ms",
        snapshot.captured_unix_ms, snapshot.sample_ms, snapshot.collection_ms
    );
    let system = &snapshot.system;
    let _ = writeln!(
        text,
        "CPU {} · RAM {}/{} · swap {} · {} logical CPUs\n",
        percent(system.cpu_percent),
        memory(system.memory_used_bytes),
        memory(system.memory_total_bytes),
        memory(system.swap_used_bytes),
        system
            .logical_cpus
            .map_or("?".into(), |value| value.to_string())
    );
    for warning in &snapshot.warnings {
        let _ = writeln!(text, "> {}", escaped(warning));
    }
    text.push('\n');
    write_node(&mut text, &snapshot.root, 0);
    text
}

fn write_node(text: &mut String, node: &Node, level: usize) {
    let indent = "  ".repeat(level);
    let collapsed = if node.omitted_children > 0 {
        " ▸"
    } else {
        ""
    };
    if let Some(process) = &node.process {
        let _ = writeln!(
            text,
            "{indent}- **{}** · {} · {} · {} · {} · {}{collapsed}",
            escaped(&node.name),
            node.reference,
            percent(process.cpu_percent),
            memory(process.resident_bytes),
            uptime(process.uptime_seconds),
            ports(process.network.as_ref())
        );
        if level == 0 {
            write_process_details(text, process, &indent);
        }
    } else {
        let _ = writeln!(
            text,
            "{indent}- **{}** · {} · {} · {} · oldest {} · {} processes{collapsed}",
            escaped(&node.name),
            node.reference,
            group_cpu(&node.totals),
            group_memory(&node.totals),
            group_uptime(&node.totals),
            node.totals.processes
        );
    }
    if node.process.is_none() || level == 0 {
        for evidence in &node.evidence {
            let evidence = if let Some(path) = evidence.strip_prefix("Application bundle: ") {
                format!("app {path}")
            } else if let Some(path) = evidence.strip_prefix("Git root from working directory: ") {
                format!("git {path}")
            } else {
                evidence.clone()
            };
            let _ = writeln!(text, "{indent}  - {}", escaped(&evidence));
        }
    }
    let mut unreadable = 0;
    for child in &node.children {
        if child.process.as_ref().is_some_and(Process::is_unreadable)
            && child.children.is_empty()
            && child.omitted_children == 0
        {
            unreadable += 1;
        } else {
            write_node(text, child, level + 1);
        }
    }
    if unreadable > 0 {
        let _ = writeln!(text, "{indent}  - **Unreadable** · {unreadable} processes");
    }
}

fn write_process_details(text: &mut String, process: &Process, indent: &str) {
    let _ = writeln!(
        text,
        "{indent}  - PPID {} · UID {}",
        process.ppid.map_or("?".into(), |pid| pid.to_string()),
        process.uid.map_or("?".into(), |uid| uid.to_string())
    );
    for (label, path) in [("cwd", &process.cwd), ("exe", &process.executable)] {
        if let Some(path) = path {
            let _ = writeln!(
                text,
                "{indent}  - {label}: {}",
                escaped(&path.to_string_lossy())
            );
        }
    }
    if let Some(network) = &process.network {
        let suffix = if network.complete { "" } else { " + ?" };
        let established = network
            .sockets
            .iter()
            .filter(|socket| socket.tcp_state.as_deref() == Some("Established"))
            .count();
        let _ = writeln!(
            text,
            "{indent}  - TCP/UDP sockets {}{suffix} · TCP established {established}{suffix}",
            network.sockets.len()
        );
        for socket in &network.sockets {
            let protocol = match socket.protocol {
                Protocol::Tcp => "TCP",
                Protocol::Udp => "UDP",
            };
            let remote = socket
                .remote
                .map_or("-".into(), |address| address.to_string());
            let state = socket.tcp_state.as_deref().unwrap_or("-");
            let _ = writeln!(
                text,
                "{indent}  - {protocol} {} → {remote} · {state} · fd {}",
                socket.local, socket.fd
            );
        }
        for reason in &network.unavailable {
            let _ = writeln!(text, "{indent}  - Network unavailable: {}", escaped(reason));
        }
    }
    if !process.unavailable.is_empty() {
        let _ = writeln!(
            text,
            "{indent}  - Unavailable: {}",
            escaped(&process.unavailable.join("; "))
        );
    }
}

fn group_uptime(totals: &Totals) -> String {
    let value = uptime(totals.oldest_uptime_seconds);
    if totals.oldest_uptime_seconds.is_some() && totals.uptime_unknown > 0 {
        format!(">={value}")
    } else {
        value
    }
}

fn ports(network: Option<&Network>) -> String {
    let Some(network) = network else {
        return "?".into();
    };
    let mut ports = BTreeSet::new();
    for socket in &network.sockets {
        let protocol = match socket.protocol {
            Protocol::Tcp if socket.tcp_state.as_deref() == Some("Listen") => "tcp",
            Protocol::Udp => "udp",
            Protocol::Tcp => continue,
        };
        if socket.local.port() != 0 {
            ports.insert(format!("{protocol}:{}", socket.local.port()));
        }
    }
    if ports.is_empty() {
        return if network.complete { "-" } else { "?" }.into();
    }
    let mut result = ports.into_iter().collect::<Vec<_>>().join(",");
    if !network.complete {
        result.push_str(" + ?");
    }
    result
}

fn group_cpu(totals: &Totals) -> String {
    partial(
        percent(Some(totals.cpu_percent)),
        totals.cpu_unknown,
        totals.processes,
    )
}

fn group_memory(totals: &Totals) -> String {
    partial(
        memory(Some(totals.resident_bytes)),
        totals.memory_unknown,
        totals.processes,
    )
}

fn partial(value: String, unknown: usize, total: usize) -> String {
    if unknown > 0 && unknown == total {
        "?".into()
    } else if unknown > 0 {
        format!(">={value} ({unknown} unknown)")
    } else {
        value
    }
}

fn percent(value: Option<f64>) -> String {
    value.map_or("?".into(), |value| format!("{value:.1}%"))
}

fn memory(value: Option<u64>) -> String {
    match value {
        None => "?".into(),
        Some(value) if value >= 1 << 30 => format!("{:.1}GiB", value as f64 / (1u64 << 30) as f64),
        Some(value) if value >= 1 << 20 => format!("{:.1}MiB", value as f64 / (1u64 << 20) as f64),
        Some(value) => format!("{:.1}KiB", value as f64 / 1024.0),
    }
}

fn uptime(value: Option<u64>) -> String {
    match value {
        None => "?".into(),
        Some(seconds) if seconds >= 86400 => format!("{}d", seconds / 86400),
        Some(seconds) if seconds >= 3600 => format!("{}h", seconds / 3600),
        Some(seconds) if seconds >= 60 => format!("{}m", seconds / 60),
        Some(seconds) => format!("{seconds}s"),
    }
}

fn escaped(value: &str) -> String {
    let mut output = String::new();
    for character in value.chars() {
        match character {
            '\\' | '*' | '_' | '[' | ']' | '`' | '<' | '>' | '|' => {
                output.push('\\');
                output.push(character);
            }
            character
                if character.is_control()
                    || matches!(character, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{2028}' | '\u{2029}') =>
            {
                let _ = write!(output, "\\u{{{:x}}}", character as u32);
            }
            character => output.push(character),
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overview_is_compact_and_unreadable_counts_preserve_pid_diagnostics() {
        use crate::tree;
        let parent = Process {
            pid: 10,
            ppid: Some(1),
            uid: Some(501),
            started_unix_us: Some(10),
            uptime_seconds: Some(300),
            name: "shell".into(),
            cwd: Some("/".into()),
            executable: Some("/bin/shell".into()),
            cpu_percent: Some(0.0),
            resident_bytes: Some(1024),
            network: Some(Network {
                complete: true,
                ..Network::default()
            }),
            ..Process::default()
        };
        let mut child = parent.clone();
        child.pid = 11;
        child.ppid = Some(10);
        child.name = "helper".into();
        child.started_unix_us = Some(11);
        let unreadable = |pid| Process {
            pid,
            name: format!("process {pid}"),
            unavailable: vec![
                "identity before observation: Operation not permitted (os error 1)".into(),
            ],
            ..Process::default()
        };
        let mut partial = unreadable(22);
        partial.resident_bytes = Some(0);
        let tree =
            tree::build(vec![parent, child, unreadable(20), unreadable(21), partial]).unwrap();
        let mut overview = String::new();
        write_node(&mut overview, &tree, 0);
        assert!(overview.contains("**Unreadable** · 2 processes\n"));
        assert_eq!(overview.matches("**Unreadable**").count(), 1);
        for hidden in [
            "pid:20",
            "pid:21",
            "PPID",
            "UID",
            "cwd:",
            "exe:",
            "TCP/UDP sockets",
            "Operation not permitted",
        ] {
            assert!(!overview.contains(hidden), "{hidden}");
        }
        assert!(overview.contains("pid:10"));
        assert!(overview.contains("pid:11"));
        assert!(overview.contains("pid:22"));
        assert_eq!(tree.totals.processes, 5);
        assert_eq!(tree.totals.memory_unknown, 2);
        let json = serde_json::to_value(&tree).unwrap().to_string();
        assert!(json.contains("pid:20"));
        assert!(json.contains("Operation not permitted"));
        let mut details = String::new();
        write_node(&mut details, &tree::select(&tree, "pid:10").unwrap(), 0);
        assert_eq!(details.matches("PPID").count(), 1);
        assert!(details.contains("cwd: /"));
        assert!(details.contains("exe: /bin/shell"));
        assert!(details.contains("pid:11"));
        details.clear();
        write_node(&mut details, &tree::select(&tree, "pid:20").unwrap(), 0);
        assert!(details.contains("Operation not permitted (os error 1)"));
        assert!(!details.contains("**Unreadable**"));
    }

    #[test]
    fn group_uptime_uses_oldest_member_and_survives_collapse() {
        use crate::{model::Process, tree};
        let processes = [Some(3600), Some(7200), None]
            .into_iter()
            .enumerate()
            .map(|(index, age)| Process {
                pid: index as u32 + 10,
                name: "sample".into(),
                uptime_seconds: age,
                ..Process::default()
            })
            .collect();
        let mut root = tree::build(processes).unwrap();
        assert_eq!(root.totals.oldest_uptime_seconds, Some(7200));
        assert_eq!(root.totals.uptime_unknown, 1);
        tree::limit_depth(&mut root, 0);
        let mut text = String::new();
        write_node(&mut text, &root, 0);
        assert!(text.contains(" · oldest >=2h · 3 processes ▸\n"));
        root.totals.uptime_unknown = 0;
        assert_eq!(group_uptime(&root.totals), "2h");
        root.totals.oldest_uptime_seconds = Some(0);
        assert_eq!(group_uptime(&root.totals), "0s");
        root.totals.oldest_uptime_seconds = None;
        root.totals.uptime_unknown = 3;
        assert_eq!(group_uptime(&root.totals), "?");
    }

    #[test]
    fn unknown_and_zero_are_distinct_and_partial_sums_are_visible() {
        assert_eq!(percent(None), "?");
        assert_eq!(percent(Some(0.0)), "0.0%");
        assert_eq!(partial("1MiB".into(), 1, 2), ">=1MiB (1 unknown)");
        assert_eq!(partial("0MiB".into(), 2, 2), "?");
    }

    #[test]
    fn process_text_cannot_inject_terminal_controls_or_markdown_lines() {
        let value = escaped("evil\n# heading\u{1b}[31m`[link](url)\u{202e}");
        assert!(!value.contains('\n'));
        assert!(!value.contains('\u{1b}'));
        assert!(!value.contains('\u{202e}'));
        assert!(value.contains("\\`\\[link\\]"));
    }

    #[test]
    fn ports_distinguish_unknown_empty_partial_and_connected_sockets() {
        use crate::model::Socket;
        assert_eq!(ports(None), "?");
        let mut network = Network {
            complete: true,
            ..Network::default()
        };
        assert_eq!(ports(Some(&network)), "-");
        network.complete = false;
        assert_eq!(ports(Some(&network)), "?");
        network.sockets.push(Socket {
            fd: 1,
            protocol: Protocol::Tcp,
            local: "127.0.0.1:1234".parse().unwrap(),
            remote: None,
            tcp_state: Some("Listen".into()),
        });
        assert_eq!(ports(Some(&network)), "tcp:1234 + ?");
        network.complete = true;
        network.sockets[0].tcp_state = Some("Established".into());
        assert_eq!(ports(Some(&network)), "-");
        network.sockets[0].protocol = Protocol::Udp;
        network.sockets[0].tcp_state = None;
        assert_eq!(ports(Some(&network)), "udp:1234");
    }
}
