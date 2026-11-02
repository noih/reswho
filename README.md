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

Settings: `reswho settings --json` shows effective values offline. Use `--config PATH`, `--sample-ms` or `--refresh-ms` to override the defaults; see the [settings contract](docs/extensions-settings.md) for the read-only JSON config format. Extension installation is not available yet.

Built-in extensions: `reswho extensions --json` lists collectors, classifiers and on-demand details offline. Availability is reported as unknown until collection; external extension installation is not available yet.
