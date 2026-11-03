# reswho — see what your resources belong to

Quickly see which apps, projects, and containers are using CPU and memory on
your Mac. reswho groups related processes so you can find what a busy process
belongs to without tracing a list of unfamiliar names by hand.

- **TUI** — Browse live usage interactively and drill into process details.
- **Markdown** — Read and share a resource snapshot as plain text.
- **JSON** — Feed structured snapshots into scripts and other tools.

Monitoring is read-only: reswho does not stop processes or start container services.

## Install

```sh
brew install noih/tap/reswho
```

Requires Apple Silicon and macOS 15 or newer. Prebuilt binaries are also on
[GitHub Releases](https://github.com/noih/reswho/releases).

Or, with a current stable Rust toolchain and Xcode Command Line Tools:

```sh
cargo install reswho --locked
```

## Use

```sh
reswho                        # Interactive resource tree
reswho --md --sort cpu        # Markdown snapshot sorted by CPU usage
reswho --json                 # JSON snapshot
```

Interactive terminals open the TUI automatically; redirected output defaults
to Markdown. Use `reswho --help` for options, `reswho --help-md` for the output
guide, or `?` inside the TUI for controls.

## Links

[GitHub](https://github.com/noih/reswho) · [crates.io](https://crates.io/crates/reswho)

## License

[MIT](LICENSE)
