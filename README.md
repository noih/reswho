# reswho — see what your resources belong to

See which apps, projects and containers are using CPU and memory on your Mac.
reswho groups related processes into a resource tree you can browse in a TUI
or export as Markdown or JSON.

Monitoring is read-only: reswho does not stop processes or start container services.

## Install

Requires Apple Silicon and macOS 15 or newer.

```sh
brew install noih/tap/reswho
```

Prebuilt binaries are also available on
[GitHub Releases](https://github.com/noih/reswho/releases).
With a stable Rust toolchain and Xcode Command Line Tools, you can install from source:

```sh
cargo install reswho --locked
```

## Use

```sh
reswho                                       # Interactive resource tree
reswho --md --sort cpu                       # Markdown snapshot sorted by CPU usage
reswho --json                                # JSON snapshot
reswho --md --from pid:1234                  # Details for a process
reswho --md --from pid:1234 --from pid:5678  # Several PIDs, one snapshot
```

Interactive terminals open the TUI; redirected output defaults to Markdown.
Use `reswho settings` to list settings and `--set KEY=VALUE` for a per-run override.
Use `reswho extensions` to list features you can enable or disable.
Settings and extensions can also be managed in the TUI.

Use `reswho --help` for options, `reswho --help-md` for the Markdown guide,
or `?` in the TUI for controls. For scripts, see the [JSON contract](docs/json-format.md).

## Contributing

Bug fixes, features, tests and documentation are welcome.
See [Contributing](CONTRIBUTING.md) for the development workflow.

## Versioning

reswho follows [EffVer](https://jacobtomlinson.dev/effver/), based on upgrade effort.
See the [changelog](CHANGELOG.md) for changes and migration steps.

## License

[MIT](LICENSE)
