# Contributing

Bug fixes, features, performance improvements, tests and documentation are welcome.

## Development

You'll need macOS, a stable Rust toolchain and Xcode Command Line Tools.

```sh
cargo build --locked
cargo run -- --no-containers
```

Follow the style of the surrounding code and keep changes focused.
When behavior changes, add or update the relevant tests and user-facing docs.
See [TESTING.md](TESTING.md) for checks suited to your change.

Keep monitoring read-only and take care with sensitive process data.
For JSON output changes, refer to the [JSON contract](docs/json-format.md).

## Pull requests

Briefly explain the problem, what changed and how you checked it.
For bug fixes, include reproduction steps when possible; screenshots are helpful
for TUI changes. Mention any compatibility changes or checks you couldn't run.
