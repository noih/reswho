use crate::model::{Network, Node, Process, Protocol, Snapshot, Totals};
use std::collections::BTreeSet;
use std::fmt::Write;

pub(crate) mod containers;
#[cfg(test)]
mod markdown_tests;

pub const JSON_SCHEMA: &str = include_str!("../schemas/reswho-12.schema.json");

pub const FORMAT_HELP: &str = r#"# Markdown reading guide — reswho-md/19

Synthetic snapshot excerpt (names, refs and values are illustrative):

```markdown
# reswho

Format: reswho-md/19 · help: reswho --help-md
Captured: 1793664000000ms Unix · sample 251ms · collection 312ms
Host CPU 8.0% · RAM 8.0GiB/16.0GiB · swap 0.0KiB · 8 logical CPUs · Σ120

## Projects

group:projects · 25.0%  300.0MiB  ⏱︎ 2h  Σ3

- demo [project] group:abcd1234 · 25.0%  300.0MiB  ⏱︎ 2h  Σ3
    > project Cargo.toml `/work/demo`
    - cargo pid:1234 · 25.0%  200.0MiB  2h  - [+1]
    - rustc pid:1236 · 0.0%  100.0MiB  5m  -
```

The project has three processes in total. cargo hides one direct child;
rustc is another visible member. Parent totals already include descendants.
The final process field is ports: here `-` means none observed.

## Report and hierarchy

Format identifies this Markdown version. Captured is Unix time in milliseconds;
sample is the measured host sampling interval; collection is capture duration.
Host CPU, RAM used/total, swap used and logical CPUs describe the whole machine,
even when a branch is selected. ΣN is the total host process count at the root.

Category headings carry a ref and totals; selected roots are paragraphs.
Nested `-` lists use four spaces per level. Indented `>` blocks are attributes
or evidence, not child resources. Paths use inline code to preserve literals.
A sole System services wrapper is flattened; the whole-machine Resources row
is omitted. Unreadable processes can be combined as `Unreadable  ΣN`, without
a ref or expandable details.

Copy a row's ref to select it. Group refs identify groups,
not display positions;
PID refs address the current PID, which may be reused. Depth counts tree edges
from the selected root (root=0), after Markdown flattening. Headings, notes and
blank lines do not consume depth. [+N] counts collapsed direct children,
including groups or processes; ΣN includes all host process descendants.
Depth hides rows without changing totals. See --help for selection examples.

## Rows, values and units

Fields after `·` have two spaces between them in raw output:

- Process: name pid:REF · CPU  RSS  uptime  ports
- Host group: name [grouping] group:REF · CPU sum  RSS sum  ⏱︎ uptime  ΣN
- Category: group:REF · the same scope totals, without a name/grouping tag
- Container: name [container] group:REF · CPU  memory  uptime  state  ports
- Container group: name [grouping] group:REF · CPU sum  memory sum  ⏱︎ uptime
  followed by active / total containers, rather than ΣN
- Resource: name [resource:kind] group:REF · label: value unit
  (up to two metrics, separated by `·`); resource groups say `independent
metrics`

Resource attributes identify the provider and optional path. Selecting a
resource shows all its typed metrics. These metrics stay outside
host/container totals.
Grouping tags include [runtime], [app], [repo], [checkout], [project], [bin],
[system] and [unknown]; they describe attribution, not extra usage.

CPU 100% means one logical core for processes and host groups; container CPU
uses one runtime/daemon logical core. Values can exceed 100%. Host CPU alone
is whole-host 0–100%. Sampling and CPU scopes differ: never add host and
container usage. Host process totals exclude the separate Containers subtree.
Memory uses binary KiB/MiB/GiB units. Process RSS is its own resident memory;
groups sum known descendant values once, but shared pages can overlap.
Container memory is runtime memory, not host RSS. Never add parent totals to
children, or sibling scopes that overlap.

Process uptime is time since start, including idle time. Group ⏱︎ is the longest
known member uptime, not active use or the group's lifetime. Container uptime
is for the current running/paused incarnation. Durations use s/m/h/d.

- `?`: unavailable, never zero.
- `-`: known absent, or not applicable for inactive container metrics.
- `>=`: partial known sum/count or oldest uptime, a lower bound.
  For example, `⏱︎ >=2h` has unknown members; `⏱︎ ?` is unavailable.
- `[+N]`: collapsed DIRECT children, distinct from total descendants ΣN.

Container active means running/paused/restarting. Created/exited/dead/stopped
usage is not applicable and does not count as unknown. Missing/unrecognized
states remain unknown; incomplete inventory or activity makes counts partial.

## Ports and selected details

Process `tcp:N` is a TCP listening port; `udp:N` is a bound UDP port, not proof
of a server. A trailing `+ ?` means partial socket coverage. Selected details
show local/remote endpoints and TCP state; socket counts deduplicate observed
TCP/UDP sockets per process, not across processes. Observations are non-atomic;
traffic, Unix sockets and inactivity history are not reported.
Container `tcp:HOST->CONTAINER` ports are published configuration, not observed
listeners or traffic. Selected container details show mounts, start time,
service and attribution candidates when available.

Selecting a PID adds details for that PID only; descendants remain summaries.
`cmd:` preserves argument order; quotes mark empty/whitespace arguments, not a
runnable shell command. `entry:` is a supported script/JAR launch argument,
not verified ownership; relative paths are not resolved against current cwd.
`[filtered]` replaces detected sensitive values. Redaction is not a security
guarantee: other values, paths and positional arguments may still be sensitive.
Environment variables and container-internal processes are not collected.

## Tags, Classification and evidence

Tags lists effective hashtags (`bin` displays as #executable). Classification
shows proposals, exclusions and source-priority decisions by dimension.
Effective app and project tags may coexist; the tree chooses its primary group
from effective app, then project, then bin tags. Nested groups show attribution;
nested processes show observed parenthood. PPID may point into another group.

`app PATH` is bundle evidence; `git PATH` is a checkout root; `repo PATH` is a
shared Git directory; `project FILE PATH` is a manifest hint; `exe PATH` is an
exact executable path. Bundle IDs are declared identities, not verified signing;
versions are on-disk metadata. Multi-path apps or multi-checkout repositories
can have shared parents; single-path/checkouts stay flat. A single-process bin
is displayed directly as a PID. Missing paths remain unknown.

`!` starts an observed error or coverage diagnostic. `~` marks explicit
inference with source and conditions, not an approximate metric. Ancestor
inference
requires matching UID/cwd and compatible start times; container bind inference
requires local paths and project markers. Remote bind paths are not checked
locally. Compose labels declare ownership. Low CPU, long uptime, paths and
attribution do not establish that a process is safe to stop.

## Further reading

--profile optionally appends an extension timing table to Markdown output.
Use --help for commands, flags, sorting and copyable examples.
See docs/markdown-format.md for structure and evidence conventions,
docs/collection-probe.md for project markers and classification boundaries,
and docs/containers.md for runtime support, API/XPC transport, sampling limits
and memory accounting. See docs/extension-protocol.md for typed resources.
For JSON, use --json-schema and docs/json-format.md.
"#;

pub fn markdown(snapshot: &Snapshot) -> String {
    markdown_with_depth(snapshot, usize::MAX)
}

pub fn markdown_with_depth(snapshot: &Snapshot, depth: usize) -> String {
    let mut snapshot = snapshot.clone();
    let detail_pid = snapshot
        .scope
        .strip_prefix("pid:")
        .and_then(|pid| pid.parse().ok());
    snapshot.project(detail_pid);
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
        "# reswho\n\nFormat: reswho-md/19 · help: reswho --help-md  "
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
    let mut reported = std::collections::BTreeSet::new();
    for record in snapshot
        .extensions
        .iter()
        .filter(|record| record.issue() && reported.insert(&record.provider))
    {
        let _ = writeln!(
            text,
            "> ! Extension {} / {}: {}",
            escaped(&record.provider),
            escaped(&record.phase),
            escaped(record.reason.as_deref().unwrap_or("provider_failed"))
        );
    }
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
    if matches!(node.grouping, Some(crate::model::Grouping::Resource)) || node.resource.is_some() {
        return format!("{} · independent metrics{collapsed}", node.reference);
    }
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

pub(crate) fn metric_text(metric: &crate::model::ResourceMetric) -> String {
    format!(
        "{}: {} {}",
        metric.label,
        metric
            .value
            .as_ref()
            .map_or("?".into(), ToString::to_string),
        metric.unit
    )
}

fn write_node(text: &mut String, node: &Node, level: usize) {
    let indent = "    ".repeat(level.saturating_sub(1));
    let bullet = if level == 0 { "" } else { "- " };
    if let Some(resource) = &node.resource {
        let _ = writeln!(
            text,
            "{indent}{bullet}{} [resource:{}] {} · {}",
            name_markdown(&node.name),
            escaped(&resource.kind),
            node.reference,
            escaped(
                &resource
                    .metrics
                    .iter()
                    .take(2)
                    .map(metric_text)
                    .collect::<Vec<_>>()
                    .join(" · ")
            )
        );
    } else if let Some(container) = &node.container {
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
    if let Some(resource) = &node.resource {
        notes.push(format!(
            "Provider {}/{} · id {}",
            escaped(&resource.provider),
            escaped(&resource.version),
            code(&resource.id)
        ));
        if let Some(path) = &resource.path {
            notes.push(format!("path: {}", code(&path.to_string_lossy())));
        }
        if level == 0 {
            notes.extend(
                resource
                    .metrics
                    .iter()
                    .map(|metric| escaped(&metric_text(metric))),
            );
        }
    }
    if let Some(process) = &node.process
        && let Some(report) = &process.tag_report
    {
        notes.push(escaped(&format!(
            "Tags {}",
            effective_tags_text(Some(report))
        )));
        notes.push("Classification".into());
        notes.extend(tag_report_lines(report).iter().map(|line| escaped(line)));
    }
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

pub(crate) fn command_text(argv: &[String]) -> String {
    argv.iter()
        .map(|arg| {
            if arg.is_empty()
                || arg
                    .chars()
                    .any(|c| c.is_whitespace() || matches!(c, '"' | '\\'))
            {
                serde_json::to_string(arg).expect("string serialization")
            } else {
                arg.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
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
    if let Some(command) = &process.command {
        if let Some(argv) = &command.argv {
            let rendered = command_text(argv);
            notes.push(if argv.is_empty() {
                "cmd: []".into()
            } else {
                format!("cmd: {}", code(&rendered))
            });
        }
        if let Some(entrypoint) = &command.entrypoint {
            notes.push(format!("entry: {}", code(entrypoint)));
        }
        notes.extend(
            command
                .unavailable
                .iter()
                .map(|reason| format!("! {}", escaped(reason.as_str()))),
        );
    }
    for details in &process.extension_details {
        for field in &details.fields {
            notes.push(format!(
                "{} · {}: {}",
                escaped(&details.provider),
                escaped(&field.label),
                code(&field.value.to_string())
            ));
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

pub fn extension_profile(records: &[crate::extensions::report::Record]) -> String {
    let mut text = "\n## Extension timings\n\nProvider | Version | Phase | Result | Duration ms | Reason\n--- | --- | --- | --- | ---: | ---\n".to_string();
    for record in records {
        let _ = writeln!(
            text,
            "{} | {} | {} | {} | {} | {}",
            escaped(&record.provider),
            escaped(record.version.as_deref().unwrap_or("unknown")),
            escaped(&record.phase),
            record.result,
            record
                .duration_ms
                .map_or("-".into(), |ms| format!("{ms:.3}")),
            escaped(record.reason.as_deref().unwrap_or("-"))
        );
    }
    text
}

pub fn effective_tags_text(report: Option<&crate::extensions::tags::Report>) -> String {
    let Some(report) = report else {
        return "Unavailable".into();
    };
    if report.effective.is_empty() {
        return "None".into();
    }
    report
        .effective
        .iter()
        .map(|entry| {
            let label = entry
                .tag
                .label
                .split_whitespace()
                .collect::<Vec<_>>()
                .join("-");
            match entry.tag.dimension.as_str() {
                "bin" => "#executable".into(),
                "app" | "project" => format!("#{}:{label}", entry.tag.dimension),
                _ => format!("#{label}"),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn tag_report_lines(report: &crate::extensions::tags::Report) -> Vec<String> {
    let mut lines = Vec::new();
    for record in &report.proposals {
        use crate::extensions::tags::Operation;
        let operation = match &record.proposal.operation {
            Operation::Add { tag } => format!(
                "add {} {} · {} · basis {}{}",
                tag.dimension,
                tag.label,
                tag.id,
                tag.basis,
                tag.path
                    .as_ref()
                    .map(|p| format!(" · {}", p.display()))
                    .unwrap_or_default()
            ),
            Operation::Exclude { selector } => format!(
                "exclude {} · tag {} · source {}",
                selector.dimension,
                selector.tag_id.as_deref().unwrap_or("all"),
                selector.source.as_deref().unwrap_or("declared sources")
            ),
            Operation::Withdraw { proposal_id } => format!("withdraw {proposal_id}"),
        };
        lines.push(format!(
            "{} · source {}/{} · proposal {}",
            operation, record.source, record.version, record.proposal.id
        ));
    }
    for decision in &report.decisions {
        lines.push(format!(
            "{}: {} · source {} · proposal {}{}",
            decision.status,
            decision.reason.replace('_', " "),
            decision.source,
            decision.proposal_id,
            decision
                .by_source
                .as_ref()
                .map(|s| format!(" · by {s}"))
                .unwrap_or_default()
        ));
    }
    if !report.diagnostics.is_empty() {
        lines.push("Tag evidence incomplete".into());
        lines.extend(report.diagnostics.iter().map(|diagnostic| {
            format!(
                "{} · {} · {}",
                diagnostic.code, diagnostic.operation, diagnostic.message
            )
        }));
    }
    lines
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
            command: Some(crate::model::ProcessCommand {
                argv: Some(vec![
                    "shell".into(),
                    "job script.sh".into(),
                    "".into(),
                    "[filtered]".into(),
                ]),
                entrypoint: Some("job script.sh".into()),
                redacted_indices: vec![3],
                ..crate::model::ProcessCommand::default()
            }),
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
            "cmd:",
            "entry:",
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
        assert_eq!(details.matches("cmd:").count(), 1);
        assert!(details.contains("cmd: `shell \"job script.sh\" \"\" [filtered]`"));
        assert!(details.contains("entry: `job script.sh`"));
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
