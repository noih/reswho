# reswho

A read-only macOS resource monitor that groups processes by application, project, and executable, with Docker container inspection.

Status: early development.

```sh
cargo run -- --tui
cargo run -- --md --sort cpu
cargo run -- --json
```

Interactive terminals open the TUI automatically; redirected output defaults to Markdown. Use `--no-containers` for host-only inspection, `--help-format` for output conventions, or `?` inside the TUI for controls.

JSON consumers: see the [data contract](docs/json-format.md), or run `reswho --json-schema` for the offline schema.
