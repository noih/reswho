# JSON contract

Use JSON for programmatic consumers. Field definitions, types and allowed values
are in [reswho-13.schema.json](../schemas/reswho-13.schema.json); this page covers
semantics that schema validation alone cannot express.

```sh
reswho --json --depth all > snapshot.json
reswho --json --from pid:1234 --from group:containers
reswho --json-schema > reswho.schema.json
```

`--json-schema` works offline without collecting resources. The schema uses
JSON Schema Draft 2020-12 and internal references only.

## Compatibility and exit status

Check `schema` before interpreting a snapshot. Ignore unknown object fields;
use the schema's enums where defined, and preserve unknown values in open strings
such as container states and memory bases. Display names, evidence and diagnostic
messages are presentation text, not identifiers or parsing interfaces.

Removing or renaming fields, changing types, units, null semantics, ref generation
or closed enums requires a new schema version. Optional fields can be additive.
Older schemas remain in `schemas/`; upgrade steps are in [CHANGELOG](../CHANGELOG.md).

| Exit | Meaning |
| --- | --- |
| 0 | All selections were found; data may still be incomplete |
| 3 | Some selections were found; JSON includes diagnostics for missing ones |
| 2 | Invalid arguments; no snapshot is collected |
| 1 | No selections matched, configuration failed, or a snapshot could not be produced |

Successful output is one UTF-8 JSON object followed by a newline. Errors go to
stderr. A write failure can leave incomplete output, so consumers must finish
parsing before using it. A closed downstream pipe is treated as normal termination.

## Selections and identity

Repeated `--from` options share one collection. `selections` preserves input
order after deduplication; each selection applies its own depth and sorting.
A missing selection has `root: null`. `nested_in` identifies overlapping selected
subtrees: their totals must not be added together.

Depth limits presentation, not collection or totals. `omitted_children` counts
hidden direct children, not all descendants. An empty visible tree does not prove
that collection was complete. Each ref is unique within the full tree, although
overlapping selections may repeat the same node.

Overview collection does not request per-process command/details or runtime disk
usage, even at `--depth all`. Explicit PID selections request only those PIDs;
selecting a group does not request every descendant's details. Repeated selections
share collection and detail work. Detailed queries remain bounded by the shared
deadline; partial or unavailable results retain diagnostics.

Refs are opaque query selectors. Copy them from output rather than reconstructing
them. Paths, classification and runtime endpoints can change group refs. PID refs
can be reused by the OS: cross-snapshot process matching requires your own host
identity plus `pid` and `started_unix_us`. A missing start time cannot establish
continuity. The snapshot has no host UUID, boot ID or persistent observation ID.
Container IDs are scoped to their runtime, endpoint and daemon; restarts can keep
the ID, so also check a valid `started_at` when matching an incarnation.

## Metrics and missing data

- `null` means unknown or not applicable; zero means a known zero. Interpret
  applicability using the resource type and state. Unknown counts cover observed
  members only; `inventory_complete: false` can also mean members are missing.
- `known_*` totals sum available measurements. With unknown members, they are
  lower bounds. Parent totals already include descendants. Collapsing a subtree
  does not change these totals.
- Process and container CPU use one logical core as 100%; `system.cpu_percent`
  uses the whole machine as 100%. RSS can include shared pages in several
  processes, so summed RSS is not physical RAM usage.
- Host `totals`, `container_totals` and generic resource metrics have separate
  scopes. Do not add container usage to host usage or infer generic-resource
  aggregation from a metric's name. Zero host totals on a container/resource node
  do not mean that resource uses nothing.
- Inactive containers have inapplicable CPU, memory and uptime; they do not
  increase metric-unknown counts. An unknown state is not an inactive state.
- Bytes are raw byte counts, percentages are numeric percentages, and time units
  follow field suffixes. Integers can exceed JavaScript's safe integer range;
  use a lossless parser when exact values matter.

A snapshot is not atomic. `captured_unix_ms` is near collection completion;
`sample_ms` describes host sampling, not every runtime's window. `collection_ms`
excludes serialization. Module timings can overlap and must not be summed to
obtain wall time. Short CPU windows can fluctuate; uptime includes idle time.

Optional process `state` is the macOS process state at identity revalidation:
`creating`, `runnable`, `sleeping`, `stopped`, or `zombie`. Missing means unknown.
`sleeping` is not an application-level idle status. `zombie` means an exited
process awaiting its parent's collection; it is not evidence of reclaimable RSS.

Optional runtime `disk_usage` contains source-qualified daemon storage estimates.
Its query has a shorter deadline than collection; a disk-query timeout retains
the runtime and container metrics, with a `docker.disk_usage` diagnostic.
For Docker, selecting the exact runtime ref requests `docker system df` once;
overview, container, and category selections do not. TUI runtime details are a
captured view refreshed with `r`, rather than a recurring disk scan. Each entry
has `kind`, `total`, `active`, `size`, and `reclaimable`. The last two fields retain
the CLI's rounded display strings (including units/percentages), not exact bytes.
Missing data is not zero. Estimates are separate from RAM totals and do not
guarantee host disk space recovery or that deletion is safe.

Container memory bases differ: Docker API values subtract applicable cgroup
inactive-file memory, Docker CLI values are additionally rounded, Apple reports
raw XPC usage, and Podman reports cgroup usage without that adjustment. These are
not interchangeable with host RSS. Published ports describe configuration, not
observed listeners or traffic; remote mount paths are not local host paths.

`metrics_scope: in_vm` and `host_resources` refer to different measurements.
Host-resource links can point outside the selected subtree; recheck process
start identity when following them. Docker configuration describes the VM;
Apple's `container_limit` describes the container, not total VM RAM. Adding Apple
`cpus` and `cpu_overhead` gives configured VM CPUs, but guest memory overhead is
unknown. Do not subtract container usage from VM usage to infer exact overhead.

## Summary hints

Optional node `summary` is the extension-selected compact display object
`{label, value}`. Both fields are presentation text, not identifiers. An absent
summary is omitted. Priority, inheritance and duplicate evidence are internal;
serializers do not select or recompute summaries. Markdown overviews use this
same selection, suppressing inherited duplicates along the displayed ancestor
chain by full source/kind/value/title/path identity, not clipped text. Hint basis
and process coverage describe provenance and aggregation, not display identity.
Selected Markdown PID/group roots show full hints instead of the compact summary.

Optional node `hints` survive summary projection. Each has `source`, `basis`,
`kind`, `value`, `processes`, and optional `path`/`title`. Process rows describe
that process only. Group `processes` is the number of distinct descendant PIDs
with that hint; compare with the group's host `totals.processes`. Multiple
candidates coexist. Hints never add usage, resources, refs or tree depth.

Project hints describe discovered cwd candidates, including when app grouping
wins. The `Project associations` category and `Project via cwd` summaries describe
an association, not exclusive ownership: a server may serve other projects.
`Project via parent cwd` identifies an inherited association. Neither establishes
active use, sharing, abandonment, or permission to stop the process.
Playwright CLI 1.64.x session names come from a verified daemon argument chain;
`ancestor_cwd` project hints additionally require project discovery and disappear
when `project` is disabled. Session names alone are not project evidence.

Codex 0.166.0 `session_log` with `open_session_log` basis identifies an observed writable
append handle to a matching session metadata record. `value` is the full stable
thread ID; `title`, when present, is the exact thread's last recorded index name.
This does not establish the foreground/current conversation, active use, or
session-specific CPU/RSS. Several handles may coexist in one daemon. Titles are
omitted when the exact ID is outside the last 64 KiB of the index, or that tail is
absent, unsafe or changing; prompts and
SQLite title fallbacks are not used. Unknown/reverted/compressed record layouts
are omitted. Current-session mapping is unsupported for Codex and Claude Code;
launch `resume` arguments, cwd, recency and Claude's potentially stale PID
registry do not prove it. Detail tag hints distinguish this unsupported guarantee
from inaccessible descriptors, conflicting/unsupported/changing log metadata,
missing safe titles and absence of a supported writable log. Ordinary summaries
omit these unavailable-reason hints.

## Attribution, privacy and diagnostics

Attribution records evidence, not proof of ownership or permission to stop a
process. Bundle IDs and Compose labels are declarations; ancestor and bind-path
associations are inferences. `launched_by` describes observed ancestry, which can
be lost after reparenting. Version metadata comes from disk and may differ from
an already-running binary after an upgrade or installation-prefix reuse.

Explicit PID selections request command and tag details only for each selected
root, not its descendants. An absent detail field means it was not requested;
requested-but-unavailable data carries diagnostics. Later selections can receive
less detail if the shared deadline is exhausted.

Command arguments are redacted using known patterns; custom positional arguments
may still contain secrets. `redacted_indices` identifies affected argv entries.
Argv is not a shell command or a trustworthy launch record. Node/Python work-name
collection also reads a limited argument prefix by default; disable it with
`extensions.node.entry_argv=false` and `extensions.python.entry_argv=false`.
Relative entry paths use the current cwd, which may differ from the launch cwd.

Use diagnostic `code`, `operation` and optional runtime `reason` for structured
handling; `message` is for display. An absent reason is unclassified, while
`unknown` means classification found no sufficient cause. `os_error` is an
observed OS error, not a guess from CLI text. `collection_note` describes a static
limit rather than a failure in this capture. Runtime `available` confirms an
inventory response, not complete stats. Socket `complete` covers listed supported
descriptors, not all possible network activity.

## Examples

The checked-in examples use synthetic data: [host](examples/host.json),
[collapsed tree](examples/collapsed.json), [containers](examples/containers.json)
and [unavailable data](examples/unavailable.json).

```sh
reswho --json --depth all | jq '
  if .schema != "reswho/13" then error("unsupported reswho schema") else . end
  | .selections[].root | select(. != null) | recurse(.children[])
  | select(.kind == "process")
  | {ref: .reference, pid: .process.pid, start: .process.started_unix_us,
     cpu: .process.cpu_percent, rss: .process.resident_bytes}
'
```

The `herdr` extension identifies session-wide server scope from a verified
process identity and supported launch arguments. Its Shared services category
does not establish active clients or their projects. Project/cwd evidence remains
in the process tags; usage is not duplicated. Selected PID details explain this
scope and report current clients as unknown.
Saved Ownership priorities still take precedence: move `extensions.herdr.role`
ahead of `project` (or reset the order) to use service grouping with an older
custom order. The service-scope evidence remains available under either grouping.
