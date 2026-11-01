use crate::model::{Network, Node, Process, Protocol, Snapshot, Totals};
use std::collections::BTreeSet;
use std::fmt::Write;

pub(crate) mod containers;
#[cfg(test)]
mod markdown_tests;

pub const JSON_SCHEMA: &str = include_str!("../schemas/reswho-8.schema.json");

pub const FORMAT_HELP: &str = "reswho-md/17 (collection prototype)\n\n\
| Row | Fields, in order |\n\
| --- | --- |\n\
| Process | name pid:REF · CPU/core  RSS  uptime  ports |\n\
| Group | name [grouping] group:REF · CPU/core sum  RSS sum  ⏱︎ uptime  ΣN |\n\
| Container | name [container] group:REF · CPU/core  memory  uptime  state  published ports |\n\
| Category summary | group:REF · CPU/core sum  RSS sum  ⏱︎ uptime  ΣN |\n\
\n\
Container groups use container counts and Docker CLI memory, not process RSS.\n\
Host process totals exclude the separate Containers subtree; never add both scopes.\n\
Docker uses the current context / DOCKER_CONTEXT / DOCKER_HOST; --no-containers skips it.\n\
Docker CLI queries share a 10s deadline and a 256-container inventory limit; partial inventory uses >=.\n\
Only the Docker adapter is implemented; Linux daemon stats are rounded CLI values (memory excludes cache).\n\
Container CPU 100% is one daemon logical core; sampling differs from host processes.\n\
Container active = running/paused/restarting; created/exited/dead metrics are not applicable (-), excluded from unknown counts.\n\
Container summaries show active / total; unknown activity or incomplete inventory keeps active counts partial.\n\
Removing or unrecognized/missing states remain unknown, not inactive. Uptime is for the current running/paused incarnation.\n\
Published tcp:HOST->CONTAINER ports are configuration, not observed listeners or traffic.\n\
Compose labels declare project ownership; ~ bind inference requires local Unix transport and project markers.\n\
Remote bind paths are not checked locally. Ambiguous candidates are shown in container details.\n\
--from group:REF shows container mounts, start time, service and attribution candidates.\n\
Runtime diagnostics omit raw CLI stderr which may contain endpoint credentials.\n\
Grouping: [runtime], [container], [app], [repo], [checkout], [project], [bin], [system], [unknown]; categories have no tag.\n\
The whole-machine Resources row is omitted; its process count is in the Host CPU summary.\n\
Other selected roots are paragraphs; child categories use ## headings with a ref/totals summary.\n\
A sole System services wrapper is flattened in Markdown before --depth is applied.\n\
Other resources use nested - lists; indented > blocks contain attributes, not child resources.\n\
Headings, notes and whitespace do not change --depth. List nesting uses four spaces.\n\
Group details stay together; blank lines separate sibling groups. Consecutive PID rows stay compact.\n\
! starts a diagnostic note with an observed error; ~ marks an explicit inference with source and conditions.\n\
Neither mark means a process is safe to stop. Low CPU or long uptime alone adds no diagnostic.\n\
Static collection limitations live here; snapshots retain current errors, deduplicated within their scope.\n\
Names, grouping tags and refs use single spaces; a middle dot separates the ref from metrics; metric fields use two spaces in raw Markdown.\n\
Names and refs are plain text; paths use inline code to preserve literal characters.\n\
A bin containing only one process is shown directly as a PID; its group ref exists only for multiple processes.\n\
Uptime is elapsed time since process start, including idle time.\n\
Group ⏱︎ is the longest known member uptime, not active use or the group's lifetime; the symbol requests text presentation.\n\
⏱︎ >=2h means at least 2h with unknown members; ⏱︎ ? means unavailable, ⏱︎ - means not applicable.\n\
CPU 100% means one logical core for processes/groups; host CPU is 0–100%.\n\
Memory uses binary units (KiB/MiB/GiB). Process rows show own RSS.\n\
Group rows sum known descendant values once; shared pages can overlap.\n\
>= marks a partial sum with unknown members; ? means unavailable, - means known absent, or not applicable for inactive container metrics.\n\
Ports: tcp:N = TCP listening port; udp:N = bound UDP port (not proof of a server).\n\
A trailing + ? means partial socket coverage. Details show local/remote endpoints and TCP state.\n\
Sockets count unique observed TCP/UDP sockets per process, not descriptors; no cross-process sum.\n\
Socket snapshots are non-atomic; no traffic, Unix sockets or inactivity history is reported.\n\
Nested groups represent attribution; nested processes represent observed parenthood.\n\
Process rows are summaries; --from pid:N shows details for the selected PID only.\n\
Unreadable rows combine processes with no readable details; they cannot be expanded.\n\
JSON retains individual processes and diagnostics, including unreadable entries.\n\
PPID is preserved even when a parent belongs to a different group.\n\
Attribution: direct App > Project; validated ancestor inference > Bin > Unknown.\n\
Evidence: app PATH = app bundle; git PATH = checkout root; repo PATH = shared Git directory.\n\
Matching declared bundle IDs share an app parent across paths; single-path apps stay flat.\n\
Bundle path refs remain stable; app parents exist only while multiple paths are present.\n\
Evidence: bundle ID = declared CFBundleIdentifier, not verified signing identity.\n\
Child bundle VERSION is the on-disk Info.plist version, not necessarily the running version.\n\
Missing or invalid bundle metadata falls back to path grouping.\n\
Projects use the nearest manifest or Git root; discovery stops at the nearest Git boundary.\n\
Markers: Cargo.toml, package.json, pyproject.toml, pom.xml, settings.gradle[.kts], build.gradle[.kts],\n\
go.work, go.mod, build.sbt, Gemfile, composer.json, pubspec.yaml, mix.exs, Package.swift,\n\
deno.json[c], CMakeLists.txt, meson.build, build.zig. Only file presence is checked.\n\
project FILE PATH = manifest hint; git PATH = Git boundary, including colocated projects.\n\
Project subdirectories stay inside their checkout; coincident project/checkout roots share a row.\n\
No marker means no project claim; cwd remains available via --from pid:N and JSON.\n\
Fallback bin groups use exact root executable paths (exe PATH), not process names.\n\
Bin totals include attached process subtrees; System paths retain the System category.\n\
Missing executable paths remain unknown. Grouping does not establish a shared task or safe shutdown.\n\
Multiple active checkouts of one repository share a project parent; a single checkout stays flat.\n\
Checkout refs remain stable; a repository parent exists only while multiple checkouts are present.\n\
Ancestor attribution is labelled inferred and requires matching UID/cwd and compatible start times.\n\
A direct app/project assignment wins; different or missing cwd prevents ancestor inference.\n\
Cwd/exe paths are hints, not proof that a process is no longer needed.\n\
--from REF selects a branch; --depth N counts edges from that branch (root=0).\n\
ΣN is the total host process count, including all descendants; Unreadable uses its combined process count.\n\
Depth limits presentation only. [+N] counts collapsed direct children, not all descendants or processes; children can be groups or processes.\n\
--sort cpu|memory|uptime|name applies to all output modes (default: memory).\n\
Numeric sorts descend within siblings, unknown/inactive values last; names use natural ascending order.\n\
Process sorts use own metrics; groups use their scope totals (partial totals use known values). Ties use ref.\n\
Sorting preserves the tree and does not combine host and container totals.\n\
Copy REF from the row. Use --depth all for unlimited expansion.\n\
\n\
```sh\n\
reswho --md --from REF --depth N\n\
```\n\
\n\
Group refs are deterministic, not display positions. PID refs query the current PID.\n\
JSON retains the original root and System services nodes; depth counts original tree edges.\n\
JSON system.cpu_percent is whole-host 0-100%; process.cpu_percent and totals.known_cpu_percent use one-core 100%.\n\
JSON totals are subtree aggregates, not extra usage; do not add parent totals to children.\n\
JSON schema reswho/8 includes structured attribution and diagnostics, raw byte counts, unknown counts and sample timing.\n\
Use --json-schema for the offline JSON Schema; see docs/json-format.md for version and identity rules.\n\
Command arguments, environment, container-internal processes and review rules are not collected.\n";

pub fn markdown(snapshot: &Snapshot) -> String {
    markdown_with_depth(snapshot, usize::MAX)
}

pub fn markdown_with_depth(snapshot: &Snapshot, depth: usize) -> String {
    let mut snapshot = snapshot.clone();
    flatten_system(&mut snapshot.root);
    crate::tree::limit_depth(&mut snapshot.root, depth);
    render_markdown(&snapshot)
}

fn flatten_system(node: &mut Node) {
    if matches!(node.grouping, Some(crate::model::Grouping::Category))
        && node.children.len() == 1
        && matches!(
            node.children[0].grouping,
            Some(crate::model::Grouping::System)
        )
    {
        let wrapper = node.children.pop().expect("single system wrapper");
        node.evidence.extend(wrapper.evidence);
        node.children = wrapper.children;
        node.omitted_children = node.omitted_children.max(wrapper.omitted_children);
    }
    for child in &mut node.children {
        flatten_system(child);
    }
}

fn render_markdown(snapshot: &Snapshot) -> String {
    let mut text = String::new();
    let _ = writeln!(
        text,
        "# reswho\n\nFormat: reswho-md/17 · help: reswho --help-format  "
    );
    let _ = writeln!(
        text,
        "Captured: {}ms Unix · sample {}ms · collection {}ms  ",
        snapshot.captured_unix_ms, snapshot.sample_ms, snapshot.collection_ms
    );
    let system = &snapshot.system;
    let _ = writeln!(
        text,
        "Host CPU {} · RAM {}/{} · swap {} · {} logical CPUs{}\n",
        percent(system.cpu_percent),
        memory(system.memory_used_bytes),
        memory(system.memory_total_bytes),
        memory(system.swap_used_bytes),
        system
            .logical_cpus
            .map_or("?".into(), |value| value.to_string()),
        if snapshot.root.reference == "group:all" {
            format!(
                " · Σ{}{}",
                snapshot.root.totals.processes,
                collapsed_marker(&snapshot.root)
            )
        } else {
            String::new()
        }
    );
    let warnings = snapshot
        .warnings
        .iter()
        .filter(|warning| !crate::collect::COLLECTION_NOTES.contains(&warning.as_str()))
        .map(|warning| format!("! {}", escaped(warning.as_str())));
    write_notes(&mut text, warnings, "");
    if snapshot.root.reference == "group:all" {
        write_notes(
            &mut text,
            snapshot
                .root
                .evidence
                .iter()
                .map(|value| evidence_markdown(value)),
            "",
        );
        write_children(&mut text, &snapshot.root, 1, true);
    } else {
        write_node(&mut text, &snapshot.root, 0);
    }
    text
}

fn blank_line(text: &mut String) {
    if !text.is_empty() && !text.ends_with("\n\n") {
        text.push('\n');
    }
}

fn write_notes(text: &mut String, notes: impl IntoIterator<Item = String>, indent: &str) {
    let mut seen = BTreeSet::new();
    let notes: Vec<_> = notes
        .into_iter()
        .filter(|note| !note.is_empty() && seen.insert(note.clone()))
        .collect();
    if notes.is_empty() {
        return;
    }
    if indent.is_empty() {
        blank_line(text);
    }
    for (index, note) in notes.iter().enumerate() {
        let newline = if index + 1 < notes.len() { "  " } else { "" };
        let _ = writeln!(text, "{indent}> {note}{newline}");
    }
    if indent.is_empty() {
        text.push('\n');
    }
}

fn collapsed_marker(node: &Node) -> String {
    if node.omitted_children > 0 {
        format!(" [+{}]", node.omitted_children)
    } else {
        String::new()
    }
}

fn group_values(node: &Node) -> String {
    let collapsed = collapsed_marker(node);
    if let Some(totals) = &node.container_totals {
        return containers::totals(&node.reference, totals, &collapsed);
    }
    format!(
        "{} · {}  {}  ⏱︎ {}  Σ{}{collapsed}",
        node.reference,
        group_cpu(&node.totals),
        group_memory(&node.totals),
        group_uptime(&node.totals),
        node.totals.processes
    )
}

fn write_node(text: &mut String, node: &Node, level: usize) {
    let indent = "    ".repeat(level.saturating_sub(1));
    let bullet = if level == 0 { "" } else { "- " };
    if let Some(container) = &node.container {
        let _ = writeln!(text, "{indent}{bullet}{}", containers::row(node, container));
    } else if let Some(process) = &node.process {
        let collapsed = collapsed_marker(node);
        let _ = writeln!(
            text,
            "{indent}{bullet}{} {} · {}  {}  {}  {}{collapsed}",
            name_markdown(&node.name),
            node.reference,
            percent(process.cpu_percent),
            memory(process.resident_bytes),
            uptime(process.uptime_seconds),
            ports(process.network.as_ref())
        );
    } else {
        let grouping = node
            .grouping
            .as_ref()
            .map_or("", |grouping| grouping.label());
        let grouping = if grouping.is_empty() {
            String::new()
        } else {
            format!(" [{grouping}]")
        };
        let _ = writeln!(
            text,
            "{indent}{bullet}{}{grouping} {}",
            name_markdown(&node.name),
            group_values(node)
        );
    }
    let mut notes = containers::notes(node, level == 0);
    if level == 0
        && let Some(process) = &node.process
    {
        notes.extend(process_details(process));
    }
    if node.process.is_none() || level == 0 {
        notes.extend(node.evidence.iter().map(|value| evidence_markdown(value)));
    }
    write_notes(text, notes, &"    ".repeat(level));
    write_children(text, node, level + 1, level == 0);
}

fn write_children(text: &mut String, node: &Node, level: usize, categories: bool) {
    if node.children.is_empty() {
        return;
    }
    if level == 1 {
        blank_line(text);
    }
    let mut unreadable = 0;
    let mut previous_group = None;
    for child in &node.children {
        if child.process.as_ref().is_some_and(Process::is_unreadable)
            && child.children.is_empty()
            && child.omitted_children == 0
        {
            unreadable += 1;
        } else if categories && matches!(child.grouping, Some(crate::model::Grouping::Category)) {
            blank_line(text);
            let _ = writeln!(
                text,
                "## {}\n\n{}",
                name_markdown(&child.name),
                group_values(child)
            );
            write_notes(
                text,
                child.evidence.iter().map(|value| evidence_markdown(value)),
                "",
            );
            write_children(text, child, 1, false);
        } else {
            let is_group = child.process.is_none() && child.container.is_none();
            if previous_group.is_some_and(|previous| previous || is_group) {
                blank_line(text);
            }
            write_node(text, child, level);
            previous_group = Some(is_group);
        }
    }
    if unreadable > 0 {
        if previous_group == Some(true) {
            blank_line(text);
        }
        let indent = "    ".repeat(level.saturating_sub(1));
        let _ = writeln!(text, "{indent}- Unreadable  Σ{unreadable}");
    }
}

fn process_details(process: &Process) -> Vec<String> {
    let mut notes = vec![format!(
        "PPID {} · UID {}",
        process.ppid.map_or("?".into(), |pid| pid.to_string()),
        process.uid.map_or("?".into(), |uid| uid.to_string())
    )];
    for (label, path) in [("cwd", &process.cwd), ("exe", &process.executable)] {
        if let Some(path) = path {
            notes.push(format!("{label}: {}", code(&path.to_string_lossy())));
        }
    }
    if let Some(network) = &process.network {
        let suffix = if network.complete { "" } else { " + ?" };
        let established = network
            .sockets
            .iter()
            .filter(|socket| socket.tcp_state.as_deref() == Some("Established"))
            .count();
        notes.push(format!(
            "TCP/UDP sockets {}{suffix} · TCP established {established}{suffix}",
            network.sockets.len()
        ));
        for socket in &network.sockets {
            let protocol = match socket.protocol {
                Protocol::Tcp => "TCP",
                Protocol::Udp => "UDP",
            };
            let remote = socket
                .remote
                .map_or("-".into(), |address| address.to_string());
            let state = socket.tcp_state.as_deref().unwrap_or("-");
            notes.push(format!(
                "{protocol} {} → {remote} · {} · fd {}",
                socket.local,
                escaped(state),
                socket.fd
            ));
        }
        notes.extend(
            network
                .unavailable
                .iter()
                .map(|reason| format!("! Network unavailable: {}", escaped(reason.as_str()))),
        );
    }
    if !process.unavailable.is_empty() {
        let mut seen = BTreeSet::new();
        let reasons: Vec<_> = process
            .unavailable
            .iter()
            .filter(|reason| seen.insert(reason.as_str()))
            .map(crate::model::Diagnostic::as_str)
            .collect();
        notes.push(format!("! Unavailable: {}", escaped(&reasons.join("; "))));
    }
    notes
}

pub(crate) fn group_uptime(totals: &Totals) -> String {
    let value = uptime(totals.oldest_uptime_seconds);
    if totals.oldest_uptime_seconds.is_some() && totals.uptime_unknown > 0 {
        format!(">={value}")
    } else {
        value
    }
}

pub(crate) fn ports(network: Option<&Network>) -> String {
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

pub(crate) fn group_cpu(totals: &Totals) -> String {
    partial(
        percent(Some(totals.cpu_percent)),
        totals.cpu_unknown,
        totals.processes,
    )
}

pub(crate) fn group_memory(totals: &Totals) -> String {
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

pub(crate) fn percent(value: Option<f64>) -> String {
    value.map_or("?".into(), |value| format!("{value:.1}%"))
}

pub(crate) fn memory(value: Option<u64>) -> String {
    match value {
        None => "?".into(),
        Some(value) if value >= 1 << 30 => format!("{:.1}GiB", value as f64 / (1u64 << 30) as f64),
        Some(value) if value >= 1 << 20 => format!("{:.1}MiB", value as f64 / (1u64 << 20) as f64),
        Some(value) => format!("{:.1}KiB", value as f64 / 1024.0),
    }
}

pub(crate) fn uptime(value: Option<u64>) -> String {
    match value {
        None => "?".into(),
        Some(seconds) if seconds >= 86400 => format!("{}d", seconds / 86400),
        Some(seconds) if seconds >= 3600 => format!("{}h", seconds / 3600),
        Some(seconds) if seconds >= 60 => format!("{}m", seconds / 60),
        Some(seconds) => format!("{seconds}s"),
    }
}

fn evidence_markdown(value: &str) -> String {
    if value.starts_with("Inferred from ancestor pid:")
        || value.starts_with("Container project inferred:")
    {
        return format!("~ {}", escaped(value));
    }
    if value.starts_with("Runtime unavailable: ")
        || value.starts_with("Repository metadata unavailable: ")
        || value == "Parent chain cycle or depth limit; displayed as a separate root"
    {
        return format!("! {}", escaped(value));
    }

    for (prefix, label) in [
        ("Application bundle: ", "app"),
        ("Git root from working directory: ", "git"),
        ("repo ", "repo"),
        ("exe ", "exe"),
        ("container project path ", "path"),
        ("compose ", "compose"),
    ] {
        if let Some(path) = value.strip_prefix(prefix) {
            return format!("{label} {}", code(path));
        }
    }
    if let Some((manifest, path)) = value
        .strip_prefix("project ")
        .and_then(|value| value.split_once(' '))
    {
        return format!("project {} {}", escaped(manifest), code(path));
    }
    escaped(value)
}

fn name_markdown(value: &str) -> String {
    let mut name = escaped(value);
    if (name.starts_with(['#', '-', '+', '=']) || name.starts_with(char::is_whitespace))
        && let Some(first) = name.chars().next()
    {
        name.replace_range(..first.len_utf8(), &format!("&#{};", first as u32));
    }
    let digits = name.bytes().take_while(u8::is_ascii_digit).count();
    if digits > 0
        && matches!(name.as_bytes().get(digits), Some(b'.' | b')'))
        && name
            .get(digits + 1..)
            .is_some_and(|rest| rest.starts_with(char::is_whitespace))
    {
        name.insert(digits, '\\');
    }
    name
}

fn control(character: char) -> bool {
    character.is_control()
        || matches!(character, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{2028}' | '\u{2029}')
}

fn code(value: &str) -> String {
    let mut safe = String::new();
    for character in value.chars() {
        if control(character) {
            let _ = write!(safe, "\\u{{{:x}}}", character as u32);
        } else {
            safe.push(character);
        }
    }
    let ticks = "`".repeat(
        safe.split(|character| character != '`')
            .map(str::len)
            .max()
            .unwrap_or(0)
            + 1,
    );
    let pad = if safe.starts_with(['`', ' ']) || safe.ends_with(['`', ' ']) {
        " "
    } else {
        ""
    };
    format!("{ticks}{pad}{safe}{pad}{ticks}")
}

fn escaped(value: &str) -> String {
    let mut output = String::new();
    for character in value.chars() {
        match character {
            '\\' | '*' | '_' | '[' | ']' | '`' | '<' | '>' | '|' => {
                output.push('\\');
                output.push(character);
            }
            character if control(character) => {
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
        assert!(overview.contains("shell [bin]"));
        assert!(overview.contains("Resources group:all"));
        assert!(overview.contains("## Unclassified\n\ngroup:unclassified"));
        assert!(!overview.contains("**"));
        assert!(!overview.contains("`pid:"));
        assert!(!overview.contains("`group:"));
        assert!(overview.contains("  > exe `/bin/shell`\n"));
        assert!(!overview.contains("  - exe /bin/shell"));
        assert!(overview.contains("shell pid:10"));
        assert!(overview.contains("helper pid:11"));
        assert!(overview.contains("Unreadable  Σ2\n"));
        assert_eq!(overview.matches("Unreadable").count(), 1);
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
        assert!(json.contains("\"grouping\":\"bin\""));
        assert!(json.contains("Operation not permitted"));
        let mut details = String::new();
        write_node(&mut details, &tree::select(&tree, "pid:10").unwrap(), 0);
        assert_eq!(details.matches("PPID").count(), 1);
        assert!(details.contains("cwd: `/`"));
        assert!(details.contains("exe: `/bin/shell`"));
        assert!(details.contains("pid:11"));
        details.clear();
        write_node(&mut details, &tree::select(&tree, "pid:20").unwrap(), 0);
        assert!(details.contains("Operation not permitted (os error 1)"));
        assert!(!details.contains("Unreadable"));
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
        assert!(text.contains("  ⏱︎ >=2h  Σ3 [+1]\n"));
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
        assert_eq!(code("/tmp/a_b`c"), "``/tmp/a_b`c``");
        assert_eq!(code("/tmp/end`"), "`` /tmp/end` ``");
        let path = code("/tmp/line\n\u{1b}\u{202e}");
        assert!(!path.contains(['\n', '\u{1b}', '\u{202e}']));
        assert_eq!(evidence_markdown("exe /tmp/a_b"), "exe `/tmp/a_b`");
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
