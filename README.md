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

Press Right on a process or resource leaf for details; `d` also opens parent-process details. Press `e` in the TUI for current extension errors and timings, then Right for provider details (Left or Esc returns to inventory, then main); opening the page does not run providers. Markdown includes compact extension errors; `--md --profile` adds timings. Snapshot JSON uses `reswho/12` with structured current reports (older schemas remain available). Discovery may save changed installation metadata, without changing settings.

Built-in and native sources propose multidimensional tags; core source priority determines effective ownership and grouping. Selected PID details show effective hashtags on the Tags row (`bin` displays as `#executable`), with proposal and decision evidence below under Classification. Native extensions can also add typed resources such as storage volumes; resource metrics keep their units and stay separate from host/container totals. Use `--from group:resources` or `d` on a resource in the TUI. `--no-containers` still permits resource collectors.

In the Extensions dialog, `Priority · highest first` orders sources: Shift-Up/Down edits the draft, Ctrl-S saves for the next capture, and Esc discards. Built-ins participate and remain installed. Priority is user configuration, independent of installation and shared setting ownership.
