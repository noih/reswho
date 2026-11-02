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

Settings: `reswho settings --json` shows effective values offline. Use `--config PATH`, `--sample-ms` or `--refresh-ms`; see the [settings contract](docs/extensions-settings.md).

`reswho extensions --json` lists built-in and installed external sources with versions and compatibility. Install a local executable manifest with `reswho extensions install PATH`; contributor setup and the isolated example are in the [extension protocol](docs/extension-protocol.md). External programs run with your user permissions.
