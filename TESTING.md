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

Container smoke verification is read-only: `cargo run -- --md --from group:containers --depth all`. It queries configured Docker and Podman endpoints and the local Apple Container service when their CLIs are present, without starting services or creating containers. Use `--no-containers` for host-only inspection; existing process/socket live tests use this flag to keep their scope isolated.

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

JSON contract tests use the dev-only `jsonschema` validator with remote resolution disabled. They validate current `schemas/reswho-12.schema.json` against synthetic examples and serialized trees, reject invalid types/node variants, and check the offline `--json-schema` CLI. Separate checks test Schema8–10 compatibility; Schema11 is retained without a compatibility-validation assertion. Classification fixtures own temporary marker directories and remove them after use; no real containers are queried. The explicit process live test also validates the emitted JSON.

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

External extension lifecycle tests own a temporary config root and cover flock-serialized mutation, atomic invalid-update rejection, corrupt state, restart/order/tombstones, canonical retention, forward uses and npm engines eligibility. Protocol fixtures compile and execute only task-owned native C binaries through the shared bounded runner, covering each capability, declared-only settings, version/identity/JSON failures, nonzero/crash, per-round suppression and next-round retry, stdout bounds, cancellation/timeouts and PID cleanup. The installed example proves custom attribution and scalar parsed details through the current snapshot Schema/Markdown/TUI; the PTY runner installs it only in its owned temporary config root and uses an owned shell for the production pipeline. Old Schema8 remains separately tested. Public settings metadata tests cover signed/fractional numbers, integer safety, homogeneous single-choice options, compatibility, invalid defaults, scalar-only values and redaction.

Phase4 external verification uses the compiled Rust example (`cargo build --example extension_example`; ordinary `cargo test` also builds examples), never a script-runtime payload. Mach-O fixtures reject scripts, malformed headers/load commands and incompatible architecture; state replacement retains explicit enablement while disabling an invalid entry point. Native supervisor fixtures kill only their owned host processes with SIGINT/SIGTERM/SIGABRT/SIGKILL, including launch and return boundaries, four concurrent providers, descendants and guard exit. They verify recorded PIDs have exited before removing isolated temp roots. Monitor initialization failure, missing executable and stdin backpressure have focused checks. Python remains repository PTY/test tooling only. No user config or runtime service is changed.

Bounded supervisor latency/RSS comparison (three rounds per mode, 15-second measurement budget, no concurrent build/measurement):

```sh
cargo test --lib containers::command::tests::measure_guard_overhead -- --ignored --nocapture --test-threads=1
```

This uses the same owned compiled 100ms no-op fixture through the bounded runner, with and without supervisor. The observed parent PID establishes zero versus one extra process. Guard RSS comes from libproc, includes shared pages and is not incremental physical memory. This is a bounded startup reference, not a live runtime or load benchmark.

Phase5 discovery fixtures own isolated config roots and native binaries. They cover deterministic batch order, stable restart/update/disable, missing/invalid updates suppressing old metadata, tombstones/reinstall, conservative legacy ownership and path conflicts, builtin protection, FIFO isolation and failed persistence retaining safe existing providers. Offline CLI discovery clears PATH and proves its native marker binary never executes. Report fixtures cover post-validation rejection, success/failure/cancellation/skips, parallel calls, nonnegative fractional milliseconds and null never-invoked durations. This phase introduced Schema10; current output uses Schema12, with Schema8–11 retained. TUI state tests preserve selection/scroll and failed current reports. The PTY extension pipeline now drops an owned native package into its temporary root; a compiled native counter entry point proves offline/status/profiling do not add provider executions, and forwards the real JSON protocol to the compiled Rust example. Existing shell/Python files are only repository test tooling. Phase4 lifecycle evidence remains applicable because the runner is unchanged.

For an extension-only PTY repair, use `python3 tests/pty_smoke.py --extensions-only`; it retains native drop-in setup, malformed sibling isolation, JSON/Markdown profile counter assertions, details/status-page transitions and finally cleanup while omitting unrelated scheduling flows.


Settings popup 的受影響 production PTY journey 可單獨執行：`cargo build --bin reswho` 後執行 `python3 tests/pty_smoke.py --settings-only`。它只用自有實體 temp root，驗證 draft／invalid／cancel／save、離線 CLI 重載、no-change、reset、同 key 衝突、malformed save failure、小終端與 pause；finally 等待 child、關閉 PTY 並移除 root。設定 unit fixtures 另覆蓋 independent-key merge、未知 key 保留、0600、FIFO／symlink 拒絕及精確長小數；worker barrier fixture 驗證 in-flight 與 next-round 的 immutable settings。原有 `--extensions-only` 與其他流程不變。

UI7／UI8 的同一個 `--settings-only` PTY journey 另驗證編輯中 Esc 只放棄目前欄位、browse Esc 放棄 popup，以及 Extensions inventory／details 的 s／q／e 隔離與兩層 Esc 返回。TestBackend 覆蓋 contextual hints、固定列位置、Unicode 尾端／實際 cursor、敏感輸入與36×10 explicit Info。`--extensions-only` 的既有 native counter oracle 保留，進出詳情改由主畫面路由；不代表每次 scoped repair 都重跑該流程。


Protocol1 conformance fixtures (`tests/fixtures/extensions/`) exercise independent manifest/request/response Schemas and the production DTO parser, including additive metadata, fractional CPU, exact integers and strict unknown settings constraints. Required-feature/capability/scope fixtures verify nonexecution and raw metadata persistence. Role fixtures cover apps/projects, provenance and unchanged PID totals; generic resource fixtures cover namespaced identity, units, nulls, bounds, scope rejection, projection and renderer behavior. This phase introduced Schema11; current output uses Schema12, with Schema8–11 retained. `--extensions-only` PTY uses the compiled native example for roles plus selected PID details and typed resource drilldown, with counter checks excluding extra profiling executions. Native entry points, children, PTYs and temp config roots stay task-owned and are reclaimed. No real runtime services or user config are touched.

Review repair fixtures (`cargo test --locked --lib repair_`) cover declared/requested-scope failure isolation, typed DTO roundtrip Schema validity, exact decimal/exponent integer decoding across protocol widths, and empty/depth-zero resource rendering. The existing dev-only jsonschema validator enables `arbitrary-precision` to match serde_json and validate full-u64 decimal representations without rounding; production dependencies and the process runner remain unchanged.

rw-38 的 tags fixtures 共用內建／原生外部分類路徑，涵蓋 source priority 反轉、來源限定排除、withdrawal／矛盾隔離、namespace／canonical ownership root、app helpers、app 移除後 project 接手及 bin 移除後不復活 raw path 分組。Settings 的真實 save/restart/conflict 與 worker barrier 驗證不可變的當輪優先序；安裝更新／新來源置底不改 shared canonical owner。TUI／MD／JSON 共同投影另驗證一般樹沒有 tag payload、只有指定 PID 詳情有 typed report、子程序仍為摘要、未要求與空集合不同。Schema12／MD19 為目前契約，Schema8–11 保留。

`cargo build --bins --examples` 後可用 `python3 tests/pty_smoke.py --priority-only` 單獨驗證真實 Extensions 排序／保存／離線重載／錯誤／36×10／導航；`--extensions-only` 保留 native pipeline，`--settings-only` 保留設定 transaction journey。視覺斷言採既有 SIGWINCH 強制完整重繪後移除 CSI 的方式，不把差異輸出字串當完整畫面；無新 terminal emulator 或依賴。所有設定使用自有實體 temp 路徑，NOFOLLOW 保護不放寬；children／PTY／temp root 在 finally 清理並確認。

rw-38 review repair fixtures 另涵蓋：同 ID 不同 bin roots 與 System／Unclassified 不碰撞；內外建共同 app／project／bin 身分合併、不同 ID 保持分離且保留 checkout metadata；native symlink alias 正規化後仍檢查敏感設定，selected JSON／MD 不洩漏；內建 app／project／bin／ancestor 顯示名稱以 UTF-8 邊界限制128 bytes，長 Unicode 路徑保留真實身分且不排除健康 sibling。外部超長 label 仍拒絕整份來源。這些修正沿用既有 runner，未重跑未受影響 PTY 或 guard／live／stress。

UI12 focused TUI fixtures 直接走 production key handlers，覆蓋主樹分支／leaf、父程序 d、Left 返回 selection／TableState offset／path scroll、resource 無程序查詢、Extensions 兩層返回保留／放棄 priority draft，以及 Settings choices／文字輸入與搜尋的路由優先序。既有 `RESWHO_UI_EVIDENCE` hook 輸出實際 TestBackend frames；執行前先建立指定 evidence 目錄，部分既有 fixtures 只負責寫檔。PTY journey 改用 Right／Left 的預期按鍵，沒有增加 provider 執行。


Podman fixtures 使用合成窄 inspect／raw counters，覆蓋完整 ID、Compose／ports／mounts、遠端 bind 不作本機歸屬、inactive／unknown、paired CPU 單位與 reset／partial、版本／能力失敗、missing CLI、deadline 保留 inventory，以及設定等待。Native nested fixture 只編譯自有 C CLI，驗證 outer host death／deadline、inner timeout／cancel 後 guard／provider／CLI／descendants 都退出；不連任何 runtime。Podman external-first 的 production install＋真實 Linux-in-Docker、離線 request→400ms→兩次 stats timing 與 builtin 真實資料證據，集中於 `docs/runtime-probes.md`，一般測試不得重建實機 fixture。

新增 builtin ID 與既有 persisted external 碰撞時，state／discovery／CLI fixtures 驗證只阻擋外部來源、單一 builtin inventory、健康 sibling 可執行、原 metadata／order／tombstone／canonical definitions／config／priority 保留、startup bytes 不變、其他來源仍可 mutation，以及新 install／drop-in 拒絕與 corruption／duplicate IDs fail closed。這些都是 isolated temp roots／native markers，不修改使用者設定或服務。

rw-39 intermittent-failure diagnosis keeps startup and behavior budgets explicit. `extensions::external::tests::timeout_cancellation_and_pipe_limits_reap_owned_process_groups` retains its 2-second dispatch deadline (including native startup), separate 2-second cancellation readiness budget, 8 MiB output limit and PID-exit assertions. Its native fixture stages a PID privately, waits for the consumer to release a file barrier, then atomically publishes the complete PID. The consumer cancels and joins before checking readiness failures; cancellation cleanup has a separate 2-second assertion. This distinguishes never-published startup failures from malformed publication and slow cancellation. It does not reset the production deadline after readiness.

`fifo_entrypoints_and_manifests_never_block_offline_lifecycle` separates the first two normal installs into a bounded 5-second `setup` phase. The three operations after FIFO creation retain the strict 1-second `FIFO check` watchdog. Each budget runs from spawn return through the remaining startup and offline operation; pipes are still captured after exit. Timeout diagnostics identify argv, elapsed time, exit status and output byte counts; a watchdog failure alone does not establish FIFO blocking. Poll errors and timeouts kill/wait the owned CLI before assertions. Its isolated config, native entrypoint copy and FIFOs are removed by the temporary-directory owner even on failure. The FIFO oracle is not relaxed; no startup warmup, automatic retry or global serialization is used. Each FIFO command still includes its own startup cost, so this boundary change does not prove every cold-start failure is eliminated. Scoped evidence and remaining causal limits are recorded in [runtime-probes.md](docs/runtime-probes.md); passing reruns do not erase a prior failure.
