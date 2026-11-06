# Changelog

## 1.1.2

### Upgrade effort

Micro: no configuration migration is required. Markdown uses `reswho-md/23`.
JSON remains `reswho/13`, with optional node `summary`, process `state`, and
runtime `disk_usage` fields; strict validators should use the updated schema.

Saved Ownership priorities are preserved. To group Herdr session servers under
Shared services with an older custom order, move `extensions.herdr.role` ahead
of `project`, or reset the Ownership order.

### Changes

- Bring compact process summaries to Markdown overviews and JSON, matching the
  TUI. Selected PID/group roots retain full hints. `--depth all` expands the tree
  without requesting detailed queries for every node.
- Put process facts, commands, cwd, and connections before classification
  evidence in the TUI and Markdown. Show native process states, including zombies,
  in all three output formats.
- Refresh TUI details without collecting a full host snapshot. PID queries sample
  the target and bounded parent metadata; runtime, container, and resource queries
  use only the selected provider. Closed queries are canceled, stale results are
  rejected, and retained overview context is marked as not refreshed.
- Identify Herdr session servers from verified identities and supported launch
  arguments. Show their session-wide service scope while retaining cwd evidence
  and counting each process once. Current clients and their projects remain unknown.
- Label cwd-based grouping as Project associations, without implying exclusive
  ownership. Service scope, process state, and uptime do not establish whether a
  process can safely be stopped.
- Show Docker disk usage and reclaimable estimates only when the exact runtime
  is selected. TUI runtime details query once on open and refresh manually with
  `r`; estimates do not guarantee host disk space recovery.
- Keep batched PID detail attachment to one tree traversal and deduplicate
  repeated selections.

### Fixes

- Preserve runtime identity and container metrics when an optional Docker disk
  query times out. Report the disk-query failure separately and reap its child
  process instead of losing the whole runtime result.

## 1.1.1

- Add concise process summaries with project, tool-role, and Playwright session hints.
- Show verified Codex session-log IDs and available titles in details.
- Improve summary readability and selection highlighting.

Session-log associations do not identify the active conversation.

## 1.1.0

### Upgrade effort

Meso: scripts may need small updates; external extensions are no longer loaded.
Interactive use keeps the default grouping, refs, and totals.

- JSON moves from `reswho/12` to `reswho/13`: use `.selections[0].root`
  instead of `.root`, and `.selections[0].selector` instead of `.scope`.
- A partially missing selection exits with 3; no matches exits with 1,
  and invalid arguments exit with 2.
- Projects and Containers are extension-provided categories. All extensions can
  be disabled; `app_bundle`, `bin`, and `command` are core sources and leave
  the extension inventory. Source and Ownership priorities are separate.
- Extensions are internal modules reviewed through PRs and shipped with the CLI.
  External discovery, executable transport, staging, signatures, trust roots and
  package/signing tools are removed. Existing external files and stale enable
  settings cannot load code; user files are left untouched.
- Schema 13 reports module IDs, capabilities, state, diagnostics and timings;
  extension version, revision, author and installation provenance are removed.
  Detected program/runtime software versions remain available.
- Downgrading: 1.0.x rejects the new top-level config fields even though the
  format version remains 1. Use a pre-upgrade backup or a separate config.

### Changes

- Query several scopes from one snapshot, with ordered selections and missing-target diagnostics.
- Show process names and versions from app bundles and install layouts;
  identify Node.js and Python work without exposing raw arguments.
- Add launched-by ancestry and VM attribution, with separate host refs,
  in-VM usage, and scoped Docker/Apple configuration.
- Save extension enablement in the TUI, with conflict
  checks and retry; apply changes to the next sample.
- Unify source status, enablement, and priority in Sources, with a separate
  Ownership tab; keep selection and scroll position after saving.
- Simplify Settings controls and hints, keep arrow keys for navigation,
  and page details by the visible height, including on small terminals.
- Use defaults for invalid saved settings; repair stale priorities on Save
  and back up malformed or unknown-version configs before replacing them.
- Explain runtime failures with structured reasons and actionable hints.
- Share per-round facts across internal modules, with generic categories,
  ownership, parent projection, precise integers and secret filtering.
- Document internal module contributions and required wiring in a short guide;
  remove the obsolete native examples and package protocol documentation.

## 1.0.1

### Upgrade effort

Micro: no user changes needed.

Updated the README with a clearer introduction, quick-start examples, and links to detailed guides.

No functional changes.

## 1.0.0

### Upgrade effort

Initial release: no prior public version to migrate.

First public release, available through GitHub Releases, crates.io and the
public Homebrew tap.

- Read-only macOS process and resource attribution, grouped by application,
  project and executable.
- Ratatui TUI, Markdown, versioned JSON and an offline JSON Schema, with sorting,
  search, scope selection and process details.
- Docker, Apple Container and Podman inspection, with partial-data diagnostics
  and no automatic runtime startup.
- Native extension discovery, installation and isolated execution; source
  priorities, multidimensional tags and typed resources.
- Settings popup, offline settings queries, `--config` and `--set KEY=VALUE`.
- Apple Silicon tar.gz binaries, SHA256 checksums and a `reswho.rb` formula for
  `noih/tap`; MIT license.

Known limitations at release:

- macOS Apple Silicon only, with deployment target 15.0. macOS 15 release Actions
  passed; this does not establish compatibility with every macOS version.
- Permissions, process exits and sampling races can leave data incomplete.
  Read-only monitoring still writes settings, extension installations and
  discovery metadata under the config root.
- Apple Container stats support the verified 1.6.1 protocol. Other versions
  retain inventory and diagnostics. Podman live validation covers nested Linux
  4.9.4 only; macOS machine, SSH, rootless and newer versions remain unverified.
- External extensions run with the user's permissions and require trust in
  their source; execution isolation is not a security sandbox.
- The Rust library API is unstable. JSON and extension protocols have their own
  versioned contracts.
- No Developer ID signing or notarization. Actual Homebrew installation and
  `brew test` remain unverified.
