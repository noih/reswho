# reswho

## Development

- macOS, Rust and Ratatui. Monitoring is read-only.
- All repository documentation, TUI text and command output must be in English.
  Discuss work with the user in concise Traditional Chinese.
- Keep documentation limited to essential user contracts and context that is
  difficult to infer from code. Do not duplicate implementation details, scripts
  or built-in help, or preserve development history in docs.
- Follow `TESTING.md`. Ordinary tests must not stop existing processes, start
  container services or modify external work.

## Core and extension boundary

- The core treats every process uniformly. It collects common process facts,
  provides basic process details, and classifies processes only as
  `Applications`, `System`, or `Unclassified`.
- Extensions own advanced features, additional classifications, and handling
  specific to a tool, runtime, framework, or process. This includes project
  discovery, session IDs and titles, tool roles, summary content and selection,
  and advanced process details.
- The core and TUI may validate, transport, aggregate, and render extension
  contributions through generic contracts. Layout, clipping, selection, and
  scrolling are generic concerns. Recognizing Codex, Claude, or Playwright,
  interpreting their metadata, and deciding what their summaries mean are not.
- Fix extension behavior in the owning extension. Do not add tool-name,
  provider-ID, or tool-specific metadata branches to the core, shared host, or
  TUI. If an extension needs a missing capability, add only the smallest generic
  contract needed to carry its contribution; keep its decisions in the extension.
- Validate metadata according to its own contract. Do not reuse command-argument
  redaction rules to discard conversation titles merely because of their syntax.
  Shared terminal escaping and generic structural validation remain applicable.

## Issue tracking

- Use Tenqo MCP for requirements, tasks, defects, progress and follow-ups.
- Workspace: `Side` (ID: `efcnbsjq1p27xow`).
- Project: `ResWho` (key: `rw`, ID: `RoNYdzMR1lkYger2JsUh1`).
- Search existing records before creating an issue. Do not use GitHub Issues or
  create mirrored records.
- If Tenqo is unavailable, report the pending update; do not switch trackers.

## Git workflow

- Work directly on `main` unless the user requests another branch.
- Commit locally. Never push; the user pushes.
- Use English commit subjects and bodies, with no AI-attribution trailers.
