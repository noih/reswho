# reswho

A read-only macOS resource monitor that groups processes by application, project, and executable, with Docker, Apple Container, and Podman inspection.

Version: 1.0.0, early development. macOS only.

## Install

The v1.0.0 release is being prepared. The commands below become available only after the corresponding GitHub Release, tap update, or crate publication.

The primary distribution is a prebuilt **Apple Silicon** binary for **macOS 15 or newer**. Download `reswho-v1.0.0-aarch64-apple-darwin.tar.gz` and its `.sha256` from [GitHub Releases](https://github.com/noih/reswho/releases), then verify before extracting:

```sh
shasum -a 256 -c reswho-v1.0.0-aarch64-apple-darwin.tar.gz.sha256
tar -xzf reswho-v1.0.0-aarch64-apple-darwin.tar.gz
./reswho --version
```

Put `reswho` in a directory on your `PATH`. Release binaries are not Developer ID signed or notarized; SHA256 checks integrity, not publisher identity. The deployment target is 15.0; macOS 15 runner verification is pending the first Actions run. Local preparation was checked on macOS 26.5.2. No prebuilt Linux or Intel binary is provided, and Intel source builds are unverified.

After `reswho.rb` is committed and pushed to the existing `noih/tap`:

```sh
brew install noih/tap/reswho
```

Secondary path, after crates.io publication (requires a current stable Rust toolchain and Xcode Command Line Tools on macOS):

```sh
cargo install reswho --version 1.0.0 --locked
```

Before publication, build this checkout with `cargo build --release --locked --bin reswho`. This package distributes a CLI; its Rust library exists for shared implementation and tests and has no stable public API commitment. See [release preparation](docs/releasing.md) and [changelog](CHANGELOG.md).

Maintainers: after reviewing a clean `main`, run `scripts/release 1.0.0` to prepare the requested version, verify, tag, push, publish the crate, wait for release assets, and commit the formula in the sibling `../homebrew-tap` developer checkout (relative to this Git root). This command publishes; it is run by the maintainer, not during local preparation. The tap push remains a separate user action.

## Usage

```sh
cargo run -- --tui
cargo run -- --md --sort cpu
cargo run -- --json
```

Interactive terminals open the TUI automatically; redirected output defaults to Markdown. Use `--no-containers` for host-only inspection, `--help` for command examples, `--help-md` for the Markdown reading guide, or `?` inside the TUI for controls.

JSON consumers: see the [data contract](docs/json-format.md), or run `reswho --json-schema` for the offline schema.

Settings: main-view `s` opens a draft popup (Enter accepts a field, Esc cancels current input or discards the popup; Ctrl-S saves, Ctrl-R resets selected). `reswho settings --json` shows effective values offline. Use `--config PATH`, `--set KEY=VALUE`; see the [settings contract](docs/extensions-settings.md).

`reswho extensions --json` lists built-in and installed external sources with versions and compatibility. Drop a native macOS binary and `manifest.json` into `<config-root>/extensions/<id>/` for startup discovery, or explicitly register a manifest with `reswho extensions install PATH`; contributor setup and the isolated example are in the [extension protocol](docs/extension-protocol.md). External programs run with your user permissions.

Press Right on a process or resource leaf for details; `d` also opens parent-process details. Press `e` in the TUI for current extension errors and timings, then Right for provider details (Left or Esc returns to inventory, then main); opening the page does not run providers. Markdown includes compact extension errors; `--md --profile` adds timings. Snapshot JSON uses `reswho/12` with structured current reports (older schemas remain available). Discovery may save changed installation metadata, without changing settings.

Built-in and native sources propose multidimensional tags; core source priority resolves effective proposals and exclusions within dimensions. The tree then chooses its primary group from effective app, project, then bin tags; app and project tags can coexist. Selected PID details show effective hashtags on the Tags row (`bin` displays as `#executable`), with proposal and decision evidence below under Classification. Native extensions can also add typed resources such as storage volumes; resource metrics keep their units and stay separate from host/container totals. Use `--from group:resources` or `d` on a resource in the TUI. `--no-containers` still permits resource collectors.

In the Extensions dialog, `Priority · highest first` orders sources: Shift-Up/Down edits the draft, Ctrl-S saves for the next capture, and Esc discards. Built-ins participate and remain installed. Priority is user configuration, independent of installation and shared setting ownership. The default built-in order is `app_bundle`, `project`, `ancestor`, `bin`, `docker`, `podman`, `apple`, `command`. Podman live evidence covers 4.9.4 in nested Linux only; macOS machine, SSH, rootless and newer versions remain unverified (see [runtime probes](docs/runtime-probes.md)).

## License

[MIT](LICENSE), copyright 2026 noih.
