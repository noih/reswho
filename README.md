# reswho

A read-only macOS resource monitor that groups processes by application, project, and executable, with Docker and Apple Container inspection.

Status: early development.

```sh
cargo run -- --tui
cargo run -- --md --sort cpu
cargo run -- --json
```

Interactive terminals open the TUI automatically; redirected output defaults to Markdown. Use `--no-containers` for host-only inspection, `--help-format` for output conventions, or `?` inside the TUI for controls.

JSON consumers: see the [data contract](docs/json-format.md), or run `reswho --json-schema` for the offline schema.

Settings: main-view `s` opens a draft popup (Enter accepts a field, Esc cancels current input or discards the popup; Ctrl-S saves, Ctrl-R resets selected). `reswho settings --json` shows effective values offline. Use `--config PATH`, `--sample-ms` or `--refresh-ms`; see the [settings contract](docs/extensions-settings.md).

`reswho extensions --json` lists built-in and installed external sources with versions and compatibility. Drop a native macOS binary and `manifest.json` into `<config-root>/extensions/<id>/` for startup discovery, or explicitly register a manifest with `reswho extensions install PATH`; contributor setup and the isolated example are in the [extension protocol](docs/extension-protocol.md). External programs run with your user permissions.

Press `e` in the TUI for current extension errors and timings; opening the page does not run providers. Markdown includes compact extension errors; `--md --profile` adds timings. Snapshot JSON uses `reswho/10` with structured current reports (older schemas remain available). Discovery may save changed installation metadata, while the core config remains read-only.
