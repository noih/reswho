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

Container smoke verification is read-only: `cargo run -- --md --from group:containers --depth all`. It queries the configured Docker endpoint and the local Apple Container service when its CLI is present, without starting services or creating containers. Use `--no-containers` for host-only inspection; existing process/socket live tests use this flag to keep their scope isolated.

TUI state and TestBackend tests cover sorting, search ancestry, filtering without changing totals, stable selection, PID reuse, unreadable summaries, the selected-path status bar, loading within the table layout, resizing, control-character escaping, and worker cancellation. The command cancellation test starts and reaps its own shell/sleep fixture; it does not invoke Docker.

Docker metadata is queried afresh on every collection. Historical refresh cost measurements and their limits are recorded in [docs/refresh-performance.md](docs/refresh-performance.md).

Direct Docker stats tests own temporary Unix socket servers, socket connections and joined server threads. They cover chunked/oversized/truncated HTTP bodies, error redaction, redirects, four-request concurrency, cancellation and deadlines, fresh paired counters, cgroup memory accounting, API negotiation, CLI fallback and restart rejection through the collector. Ordinary tests still never contact real Docker. The production client uses libcurl multi in the collecting thread, with bounded responses parsed one batch at a time; sockets and handles are dropped on success, failure or cancellation.

Explicit read-only Docker phase timing uses the existing command runner and collector, with no cache or service lifecycle changes:

```sh
cargo test --locked --lib containers::measure::measure_docker_collection -- --ignored --nocapture --test-threads=1
```

This ignored measurement runs three sequential rounds within a 40-second budget, preserving the collector's per-round 10-second deadline. It records anonymous per-command timings, batch sizes, output byte counts and diagnostic codes, then stops on incomplete collection. It also measures `docker --version` as a local CLI startup/handling reference, not pure process-spawn cost. No names, IDs, paths, endpoints or raw stderr are printed. Owned CLI processes and pipes are reclaimed by the existing runner; only summary timings should be retained. Do not run other builds or benchmarks concurrently with this measurement.

On a supported Unix endpoint, `stats.api` includes version negotiation, both one-shot passes and the sampling wait. Its `output_bytes` is 0 because this harness does not count API response bytes. `non_command_ms` excludes that timed API stage as well as CLI commands. Historical CLI-only timings remain documented with their original revision.

To measure the complete host + configured container runtimes capture with the production snapshot path, prebuild once and run the explicit three-round check within its 40-second measurement budget:

```sh
cargo test --locked --lib --no-run
cargo test --locked --lib containers::measure::measure_full_snapshot -- --ignored --nocapture --test-threads=1
```

It prints anonymous capture/runtime durations, counts, metric sources and container completeness. It stops on container diagnostics or missing active metrics. Host warning counts include static collection notes; they do not establish completeness of individual process data. Capture timings exclude rendering and CLI startup. The same read-only and resource cleanup rules apply; do not run another benchmark concurrently.

The separate, read-only stats experiment requires Python 3.9+, Docker CLI and a selected Unix endpoint backed by Linux Engine API 1.41+:

```sh
python3 scripts/measure_docker_stats.py
```

It compares CLI no-stream, direct non-streaming API, and two fresh one-shot samples separated by 250ms. It schedules three rounds with rotated mode order, at most eight running containers and four concurrent API requests. Its total budget is 60 seconds; CLI calls have a 10-second timeout and socket I/O a 5-second timeout, both bounded by the remaining budget. Each comparison checks identity/state before and after stats; incomplete data stops the experiment with a nonzero exit. Existing workloads can change during measurement, so a stopped run is not a passing three-round benchmark. API timings exclude inventory and identity checks. Summaries contain no container identifiers or raw responses. All data remains in memory; CLI children are reaped, and pipes, HTTP responses, sockets and executor workers are reclaimed. The script does not change production collection or start services. Do not run concurrent builds or benchmarks; retain incomplete results and their limits alongside successful comparisons in the performance notes.

Explicit macOS PTY smoke verification requires Python 3:

```sh
cargo build --bins --examples
python3 tests/pty_smoke.py
```

This creates owned PTYs and reswho processes with `--no-containers`. It checks automatic TUI selection, keyboard controls, resize, missing scope errors, q/Ctrl-C exits, and spinner updates plus terminal restoration while a five-second sample is pending. It verifies synchronized-update begin precedes alternate-screen entry, the complete loading layout precedes synchronized-update end, and no synchronization remains open after exit. Raw terminal output may use cursor movement between words; labels are checked separately. These protocol checks do not prove flicker-free rendering in every terminal emulator or inject initialization I/O failures. All child processes and PTY descriptors are reclaimed in `finally`.

JSON contract tests use the dev-only `jsonschema` validator with remote resolution disabled. They validate `schemas/reswho-8.schema.json` against synthetic examples and serialized trees, reject invalid types/node variants, and check the offline `--json-schema` CLI. Classification fixtures own temporary marker directories and remove them after use; no real containers are queried. The explicit process live test also validates the emitted JSON.

Shared sorting tests cover own process metrics versus descendant totals, unknown and inactive values, container scope, natural name order, unchanged references/totals, and Markdown/JSON ordering. CLI argument tests cover every sort mode; the PTY smoke test checks `--sort name` sets the initial TUI order while automatic TTY detection still applies.

PID command tests cover bounded KERN_PROCARGS2 decoding, argc-only parsing (no environment output), 32/64-bit alignment and leading empty argv[0], empty arguments, malformed/oversized data, conservative redaction, launch targets and identity invalidation. JSON contract tests validate the optional reswho/8 command extension; Markdown checks keep commands out of summaries and preserve argument boundaries in selected-PID details.

The existing process live test also checks selected command collection and its absence in group queries. `live_command_redacts_arguments_and_never_outputs_environment` creates one owned shell blocked in the builtin `read` on a piped stdin. Only synthetic secrets are passed, and JSON/Markdown must not expose them or the environment sentinel. The shell is killed and reaped by the fixture guard on both success and assertion failure; no child service is created. `live_script_entrypoint_preserves_literal_path` owns a temporary script with a spaced filename and a shell blocked on stdin; it verifies the literal launch path and removes the script directory after reaping the shell. Run the affected live tests explicitly:

```sh
cargo test --test live -- --ignored --test-threads=1 --skip live_tcp_udp_ipv4_ipv6_endpoints_and_duplicate_descriptors
```

TUI process-detail tests cover detail-page key routing and independent scrolling, preserved table offsets, queued on-demand command selection through the single worker, pause/manual refresh, close/reopen, late results for another PID, missing/unknown/reused identities, collection and command failures, small layouts and control-character escaping. Page rendering checks cover a single footer, retained host summary, compact attribution, non-repeated PID, label/value colors, complete Unicode paths with indented wrapping, and scrolling by visual lines. The PTY runner also owns one shell blocked in builtin `read`, opens its details, verifies loading, `[filtered]` output and manual-refresh feedback, resizes, closes/reopens and quits while command collection is pending. It reaps both the fixture and TUI process and closes stdin/PTY handles in `finally`; it never contacts Docker or starts services.

Refresh controls remain visible whenever refresh is available, with feedback only for manual requests. TUI tests cover both pages, completion/failure, repeated requests during collection, unchanged pause state, command errors, unavailable identities and help/search key isolation.

Apple Container tests use synthetic inventory/counters and the shared JSON Schema: active/stopped/unknown states, version gating, ports, bind attribution, secret omission, duplicate/malformed/limited inventory, restart/disappearance, partial stats, actual sample duration, four-request concurrency, cancellation and deadline cleanup. They do not contact the Apple service. The XPC bridge is compiled on macOS using the existing cc build toolchain; no runtime helper is installed.

Explicit native callback/response lifetime checks (no service or container required):

```sh
apple_test_dir=$(mktemp -d /private/tmp/reswho-apple-native.XXXXXX)
trap 'rm -rf "$apple_test_dir"' EXIT
xcrun clang -Wall -Wextra -Werror -std=c11 -fblocks -fsanitize=address tests/apple_xpc_native.c -o "$apple_test_dir/check"
"$apple_test_dir/check"
```

This owns only its temporary compiler output and synthetic XPC values. It checks payload types/size, error replies, pending polling, and late callback release racing with the caller. Real Apple service integration is a separately authorized fixture run; results and scope are recorded in [runtime-probes.md](docs/runtime-probes.md). Ordinary tests never start runtimes. Apple 1.6.1 is the only enabled XPC wire version; other versions keep inventory with an unsupported-stats diagnostic.

Settings fixtures cover ordered atomic registration, compatible defaults, contract conflicts, namespaces/uses, typed values and bounds, source precedence, unknown keys, secret redaction, versioned read-only config loading and offline CLI queries. The PTY runner uses its own temporary config root and owned shell fixtures to verify configured sampling, automatic refresh, waiting, pause and manual refresh; all are removed/reaped in finally. Schedule eligibility is also tested with synthetic time, without sleeping.

Extension fixtures exercise the common dispatcher with synthetic providers: concurrent collection with shared resolved sampling settings, failure and duplicate-container isolation, PID/start/path proposal validation, unchanged PID totals and classification precedence, selected command details with core redaction, JSON Schema and Markdown output, and TUI detail rendering. Offline extension CLI tests clear PATH and use a temporary config root; no runtime services are queried. Existing Docker/Apple transport fixtures remain unchanged.

External extension lifecycle tests own a temporary config root and cover flock-serialized mutation, atomic invalid-update rejection, corrupt state, restart/order/tombstones, canonical retention, forward uses and npm engines eligibility. Protocol fixtures compile and execute only task-owned native C binaries through the shared bounded runner, covering each capability, declared-only settings, version/identity/JSON failures, nonzero/crash, per-round suppression and next-round retry, stdout bounds, cancellation/timeouts and PID cleanup. The installed example proves custom attribution and scalar parsed details through Schema9/Markdown/TUI; the PTY runner installs it only in its owned temporary config root and uses an owned shell for the production pipeline. Old Schema8 remains separately tested. Public settings metadata tests cover signed/fractional numbers, integer safety, homogeneous single-choice options, compatibility, invalid defaults, scalar-only values and redaction.

Phase4 external verification uses the compiled Rust example (`cargo build --example extension_example`; ordinary `cargo test` also builds examples), never a script-runtime payload. Mach-O fixtures reject scripts, malformed headers/load commands and incompatible architecture; state replacement retains explicit enablement while disabling an invalid entry point. Native supervisor fixtures kill only their owned host processes with SIGINT/SIGTERM/SIGABRT/SIGKILL, including launch and return boundaries, four concurrent providers, descendants and guard exit. They verify recorded PIDs have exited before removing isolated temp roots. Monitor initialization failure, missing executable and stdin backpressure have focused checks. Python remains repository PTY/test tooling only. No user config or runtime service is changed.

Bounded supervisor latency/RSS comparison (three rounds per mode, 15-second measurement budget, no concurrent build/measurement):

```sh
cargo test --lib containers::command::tests::measure_guard_overhead -- --ignored --nocapture --test-threads=1
```

This uses the same owned compiled 100ms no-op fixture through the bounded runner, with and without supervisor. The observed parent PID establishes zero versus one extra process. Guard RSS comes from libproc, includes shared pages and is not incremental physical memory. This is a bounded startup reference, not a live runtime or load benchmark.
