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
