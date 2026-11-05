# Verification

Extensions are compiled internal modules. Use focused checks for the changed
contract, then the integration gates required by the task. Reuse valid results
for unchanged behavior; documentation-only changes need documentation checks.

## General checks

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked -- --test-threads=1
```

Run general tests serially, as local release gates and macOS CI do. Subprocess
startup can contend in parallel; serial success does not prove the parallel
suite passes. Keep existing deadlines and PID-readiness assertions.

Ordinary tests use synthetic data, in-process module seams, owned short-lived
processes and temporary Unix sockets. They must not contact real runtimes,
start container services, stop existing processes or modify external work.
Native process inspection and Unix socket binding need a permissive macOS
environment; report sandbox EPERM as a failed or unavailable check, never a pass.

Use the existing fixtures and shared target cache. Reap every owned child,
close handles and remove temporary roots even on assertion failure. Verify
identity before signalling a leftover; do not clean up another task's resources.
`target/` is a reusable reswho project cache, retained for the next development
phase; inspect its size at phase boundaries and remove it only when unused.

## CLI and output checks

For CLI/output changes:

```sh
cargo test --locked --test cli --test json_contract -- --test-threads=1
```

JSON tests validate examples and output with remote schema resolution disabled.

## TUI smoke checks

```sh
cargo build --locked --bin reswho
python3 tests/pty_smoke.py --priority-only
python3 tests/pty_smoke.py --settings-only
```

For changes spanning the whole terminal flow, use `python3 tests/pty_smoke.py`.
`--extensions-only` selects the same current priority journey. Journeys use
`--no-containers`, owned config roots and fixtures; finally blocks wait for
children, close PTYs and remove roots. They cover controls, resize, restoration,
settings/recovery and priority persistence. They do not install extensions or
prove flicker-free rendering in every terminal emulator.

## Explicit live and diagnostic checks

```sh
cargo test --locked --test live -- --ignored --test-threads=1
```

Live tests own temporary marker/header/binary/script fixtures, processes and
loopback sockets, then reap/close/remove them. They check native process/socket
inspection, selected command redaction, entry names and versions without
contacting container services. Ordinary tests leave these ignored.

Native Apple callback/response lifetime checks need only synthetic XPC values:

```sh
apple_test_dir=$(mktemp -d /private/tmp/reswho-apple-native.XXXXXX)
trap 'rm -rf "$apple_test_dir"' EXIT
xcrun clang -Wall -Wextra -Werror -std=c11 -fblocks -fsanitize=address tests/apple_xpc_native.c -o "$apple_test_dir/check"
"$apple_test_dir/check"
```

Ignored lifecycle stress, runtime-service live checks and timing experiments
are opt-in. Define the runtime versions and workload to cover before running them.
Read-only container smoke (`cargo run -- --md --from group:containers --depth all`)
queries configured endpoints without starting them; use `--no-containers` for
host-only work. Do not run concurrent builds or benchmarks during measurements.

## Performance measurements

Build before measuring; do not run builds or other benchmarks concurrently.
Keep the same build profile, sampling interval and collection scope when
comparing revisions. Alternate revisions under comparable workloads and report
sample counts, workload sizes, diagnostics and missing metrics alongside timings.

Distinguish full command wall time, `collection_ms`, runtime/module timings and
TUI CPU time. Collection includes the sampling wait; CLI wall time also includes
startup and output encoding. `--from` and `--depth` limit presentation, not the
full collection. Use `--profile` and extension `argv` counters to inspect work;
do not infer argument-read counts from the number of recognized processes.

Host process CPU/RSS and in-VM container metrics have different scopes. Client
CPU measurements do not include daemon/VM costs. Short samples and changing
workloads cannot establish a stable speedup or a release performance guarantee.
Keep only anonymous summaries and clean up owned measurement resources.

## Release and documentation checks

Offline release checks use `python3 tests/release_package.py`,
`python3 tests/release_script.py`, `bash -n scripts/release` and `actionlint`.
They do not publish or update a real tap. Actual Actions, crates.io uploads and
Homebrew installation require separate verification by the release owner.

For documentation-only changes, compare claims and commands with source, check
Markdown relative links/anchors and `git diff --check`, and parse affected JSON
examples. If a file is moved, check include_str paths and Cargo include coverage.
Do not run tests/build/PTY solely because a documentation commit is next.
