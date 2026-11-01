# Verification

The collection prototype uses Rust tests for native counter calculations, classification, tree projection, output escaping, and CLI contracts. Markdown structure tests use the dev-only pulldown-cmark parser to verify headings, lists, blockquotes, hard breaks, and literal paths. Docker adapter tests use fake CLI responses for context/host selection, remote/local attribution, missing metadata, restart races, limits and partial data. Subprocess tests own and reap short-lived shell/sleep fixtures to verify timeouts and diagnostic handling. Ordinary tests never contact Docker. These are technical spike checks, not acceptance of the full planned product.

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

Live macOS verification is explicit:

```sh
cargo test --test live -- --ignored --test-threads=1
```

The process live test creates a temporary Git marker and one owned `sleep` process, checks PID/PPID, working directory, counters, project classification, stable group lookup and Markdown output, then replaces its owned Git marker with go.mod to verify non-Git classification of the same process. The process is killed and reaped before the temporary directory is removed. The socket live test holds its own ephemeral loopback TCP/UDP sockets, verifies IPv4/IPv6 endpoints, listening/established TCP states and duplicate-descriptor handling, then drops all handles. Neither test touches existing services or containers. Restricted environments may deny native process inspection; report that failure rather than treating it as passing.

Build artifacts in `target/` are the project's reusable development cache, owned by the current reswho task and retained for the next implementation phase. Review its size at phase boundaries; clean it only when no build or test is using it. No collector daemon or persistent observation database is created.

Container smoke verification is read-only: `cargo run -- --md --from group:containers --depth all`. It queries the configured Docker endpoint without starting services or creating containers. Use `--no-containers` for host-only inspection; existing process/socket live tests use this flag to keep their scope isolated.

TUI state and TestBackend tests cover sorting, search ancestry, filtering without changing totals, stable selection, PID reuse, unreadable summaries, the selected-path status bar, loading within the table layout, resizing, control-character escaping, and worker cancellation. The command cancellation test starts and reaps its own shell/sleep fixture; it does not invoke Docker.

Docker metadata is queried afresh on every collection. Historical refresh cost measurements and their limits are recorded in [docs/refresh-performance.md](docs/refresh-performance.md).

Explicit macOS PTY smoke verification requires Python 3:

```sh
cargo build
python3 tests/pty_smoke.py
```

This creates owned PTYs and reswho processes with `--no-containers`. It checks automatic TUI selection, keyboard controls, resize, missing scope errors, q/Ctrl-C exits, and spinner updates plus terminal restoration while a five-second sample is pending. All child processes and PTY descriptors are reclaimed in `finally`.

JSON contract tests use the dev-only `jsonschema` validator with remote resolution disabled. They validate `schemas/reswho-8.schema.json` against synthetic examples and serialized trees, reject invalid types/node variants, and check the offline `--json-schema` CLI. Classification fixtures own temporary marker directories and remove them after use; no real containers are queried. The explicit process live test also validates the emitted JSON.

Shared sorting tests cover own process metrics versus descendant totals, unknown and inactive values, container scope, natural name order, unchanged references/totals, and Markdown/JSON ordering. CLI argument tests cover every sort mode; the PTY smoke test checks `--sort name` sets the initial TUI order while automatic TTY detection still applies.
