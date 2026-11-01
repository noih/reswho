# Verification

The collection prototype uses Rust tests for native counter calculations, classification, tree projection, output escaping, and CLI contracts. Markdown structure tests use the dev-only pulldown-cmark parser to verify headings, lists, blockquotes, hard breaks, and literal paths. These are technical spike checks, not acceptance of the full planned product.

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
