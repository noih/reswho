# 發佈

## 分工與邊界

本專案直接在 `main` 工作。準備者只完成本地 commit 與獨立 review，不執行任何真實發佈；維護者審核後自行執行：

```sh
scripts/release 1.0.0
```

參數決定 package version 與 tag。這是具有發佈權限的使用者入口，會執行普通 SSH `git push`，保留 passphrase gate；不 bypass hook、不調整認證、不 fetch／pull／clone。不要在代理的準備或測試工作階段執行真實入口。

腳本先確認乾淨 main、指定 SSH origin、remote main 為本地祖先、全新 local/remote tag 與 release、gh auth、工具與既有乾淨 `noih/tap` checkout。tap origin 必須為 `noih/homebrew-tap`，本地 HEAD 須與 `gh api` 查到的 remote branch SHA 相同；若不存在／漂移，發佈前停止，由維護者自行設定或協調，不自動安裝 tap 或同步其他 repo。403 不當成不存在。

全部預檢通過後，先準備 Cargo.toml／Cargo.lock／同樹 example engines 的完整新內容，再逐檔 replace；寫入例外會還原原內容並刪除 staging 檔，不宣稱跨檔 power-loss transaction。拒絕倒退版本。同版本初次1.0.0允許不產生空 bump commit；仍完整驗證並建立新的 annotated tag。example range 使用目前版本至下一個 major，與同樹編譯的例子一致。測試 helpers 讀 `CARGO_PKG_VERSION`，不以首發版本綁死相容性。

依序完成 fmt、clippy、串行一般測試、兩組離線腳本測試及 Cargo dry-run，才本地 commit/tag、push main/tag、執行真實 `cargo publish --locked`。使用者的 Cargo 自行取得認證；腳本不讀、不印 credential 檔或 token。認證是否有 upload 權限仍須真實發佈證明。

推送 release tag 是 GitHub Release 的發佈觸發：workflow 先建立 draft，只有建置、smoke、checksum、formula 與上傳完整性全部成功，才自動公開。不另設手動公開步驟。首次準備不建立 tag，也不執行任何遠端發佈。

## 平台決策

第一版僅建置 `aarch64-apple-darwin`，使用 `macos-15` runner 與 `MACOSX_DEPLOYMENT_TARGET=15.0`。XPC bridge 使用的 `xpc_connection_activate` 在 SDK 標註 macOS 10.12 起可用；其他橋接使用 XPC、kqueue、proc、mach 與 POSIX API。curl-sys 在目前停用預設 features 的 Apple 路徑直接連結系統 libcurl。15.0 是保守的發佈基線，不是推導出的最低可執行 OS；本機 macOS 26.5.2 的成功不能取代 macOS 15 實際測試。Apple Container 本身的系統需求也不等同 reswho host-only 的需求。

打包腳本會拒絕非 arm64、非15.0 deployment target，以及非 `/usr/lib/` 或 `/System/Library/` 動態連結，避免意外依賴 Homebrew 絕對路徑。預編譯版不宣稱 Linux／Intel 支援；Intel source build 也未驗證。當前 Rust library 是 CLI 與測試共用的實作；保留既有 modules，停用其生成文件，不承諾穩定 public Rust API。

官方參考：[GitHub macOS runners](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)、[checkout v7.0.1](https://github.com/actions/checkout/releases/tag/v7.0.1)、[Homebrew formula cookbook](https://docs.brew.sh/Formula-Cookbook)。workflow 固定 checkout commit SHA，且不保留 checkout credentials。CI 只有 contents read；release job 因建立與公開 Release 需要 contents write，不使用跨 repo token。

## 本地 gate

更新 Cargo.toml、Cargo.lock 自身版本與 changelog；tag 必須是無前導零的穩定 `vX.Y.Z`，且與 package version 完全相符。一般 gate 依序執行，不與 native fixtures 平行建置，不執行 ignored live/stress 或啟動 runtime：

```sh
cargo fmt --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked -- --test-threads=1
python3 tests/release_package.py
bash -n scripts/release
python3 tests/release_script.py
actionlint
cargo package --list --locked --offline
cargo package --locked --offline
MACOSX_DEPLOYMENT_TARGET=15.0 cargo build --release --locked --target aarch64-apple-darwin --bin reswho
python3 scripts/package-release v1.0.0 target/aarch64-apple-darwin/release/reswho target/release-dist-v1.0.0
ruby -c target/release-dist-v1.0.0/reswho.rb
(cd target/release-dist-v1.0.0 && shasum -a 256 -c ./*.sha256)
```

未 commit 的準備內容可暫用 `--allow-dirty` 進行 package／dry-run，須記錄此差異並確認最終 commit 包含同一份內容。`cargo publish --dry-run` 會建置實際 `.crate` 解包內容；不能加 `--no-verify`。本次 Cargo 1.99.1 的 offline dry-run 因需要 HTTP 而拒絕；`cargo package --locked --offline` 仍可獨立驗證實際 tarball。準備時另以無 credentials/config 的 temporary Cargo home（只共用 registry cache，移除 child env 的 `CARGO_REGISTRY_TOKEN` 與 `CARGO_REGISTRIES_CRATES_IO_TOKEN`，不讀取其值）執行一次 online dry-run。成功 dry-run 仍不能證明認證、token 權限或真實 upload。不要讀取 Cargo credentials，不需要 token；真正 crate 發佈由維護者執行的 `scripts/release VERSION` 呼叫 `cargo publish --locked`；GitHub tag workflow 本身不會發佈 crate。

Cargo include 清單包含原始碼、`build.rs`、C bridges、schemas、tests／fixtures、examples、docs、README、changelog 與 LICENSE。檢查 `cargo package --list` 及 `target/package/reswho-1.0.0.crate`：尤其 `src/containers/guard.c`、`src/containers/apple/xpc.c`、`schemas/reswho-12.schema.json`、所有 `include_str!` 路徑。`[lib] doc=false` 不會移除 library target，亦不影響 integration tests。

打包腳本使用 Python 3 標準函式庫與 macOS 的 `lipo`／`otool`；只傳 tag 時僅驗證版本。傳 binary 與新的 output directory 時先驗證 linkage、架構、deployment target，再用自己的 temporary HOME/config root 執行 `--version`、`--help`、`settings --json`、`extensions --json`。不採樣程序／容器、不執行 extension；暫存設定自動清除。產物為 tar.gz（binary、LICENSE、README）、相對檔名 SHA256 與精確代入 tag／checksum 的 `reswho.rb`。既有 output directory 直接拒絕，不覆寫先前證據。

## Tag workflow 與失敗恢復

維護者推送 reviewed commit 與對應 tag 前，確認 tag 指向預期 commit；不要推送未審核的 release workflow。workflow 每個 tag 串行且不中斷既有 run；先跑 fmt／clippy／一般測試與打包單元檢查。GitHub API 只有明確404才視為不存在，其他讀取錯誤直接失敗。任何既有 release（含 draft）都拒絕重用，`--verify-tag` 避免意外建立遠端 tag。

draft 建立後才建置 release binary；檢查與打包成功後以不含 `--clobber` 的 `gh release upload` 一次上傳三個 assets，再確認仍為 draft 且 asset 名稱完整吻合，才自動公開。不要在 workflow 執行中手動公開或修改 draft。

失敗恢復界線：

1. draft 尚未建立：修正執行環境後完整 rerun。若需要改 source／workflow，使用新版本與新 tag，不移動既有公開 tag。
2. draft 已建立但尚未公開：維護者核對 tag commit、`isDraft=true`、assets 未被其他流程使用後，刪除該 unpublished draft（連同部分 assets，保留 tag），再 **Re-run all jobs**。不要只重跑 upload，也不要覆寫單一 asset。
3. release 已公開：不能以同版本重建或替換 tarball／checksum／formula；修正版本後使用新 tag。若公開狀態不明，先停止並確認，不自動刪除。

重建可能改變 tar metadata 與 SHA256，所以只允許上述未公開 draft 的整組恢復。tag workflow 不寫入 tap，也不發佈 crate；使用者的 `scripts/release` 則串接這些步驟。避免同時執行兩個 release scripts。

## Tap 與公開後驗證

`scripts/release` 在 crate 發佈後，最多等待5分鐘尋找同 tag＋commit SHA＋push event＋release workflow 的 run，再以30分鐘 timeout 觀看該 run。成功後透過 public release URLs 下載三個 assets（不依賴 Contents read-only token 能否讀 release API），驗證 archive 檔名／SHA256、成員清單及 formula 與 reviewed template／tag／checksum 完全相同，再檢查 Ruby syntax。

再次確認 tap HEAD、工作樹與 remote SHA 未變後，才複製到既有 `Formula/reswho.rb`。停用 Homebrew auto-update 執行 style／readall／strict audit，完成 tap 本地 commit，只印出待維護者執行的 tap push。沒有跨 repo token；不自動 tap push。

維護者仍需在 macOS 15 Apple Silicon 做真實安裝與 `brew test reswho`，審核 tap 本地 commit 後自行推送。只有 tap 公開後才能宣稱 `brew install noih/tap/reswho` 可用。Ruby syntax 驗證不能取代真實 Homebrew 測試。檢查 public download、Actions 結果及 crates.io 實際安裝後才更新 README 的待發佈文字。

保留證據時列出本機 OS／Rust 版本、命令與 exit status、失敗與 scoped rerun、未執行項目、temp cleanup、`target/` 大小。`target/` 是 reswho 可重用 cache，保留到下一階段邊界再檢查大小；不要每次 gate 新建整套大型 cache。

## 使用者腳本的部分失敗

腳本會顯示失敗階段，不自動刪 tag、撤銷 crate、重建公開 assets 或重跑自身：

- push 前 gate 失敗：只有本地版本檔可能已變更，檢查／修正後自行 commit 或還原。若本地 tag 已建立，先確認 remote 狀態；不要盲目重跑腳本。
- push 失敗：main 與 tag 可能部分成功，使用 gh 核對 remote SHA/tag，再由維護者決定如何完成同一個已審核 push，不移動現有 tag。
- crate publish 失敗／結果不明：tag workflow 可能仍會成功公開 binary。先核對 crates.io 該版本狀態；確定尚未成功才由使用者從該 tag commit 單獨執行 `cargo publish --locked`，不要再次執行整支腳本或假設 upload 已失敗。
- Actions 失敗：依前述 unpublished draft 的整組恢復規則操作；若需要改 source，發新版本。
- Actions 成功但 assets／tap 階段失敗：已公開版本保持不變。重新下載該版本三個 assets 並驗證，協調 tap 的乾淨狀態後人工續做 formula copy／checks／commit；已存在的 tap commit 只需 review 後自行 push。不要重跑 release 或覆寫公開 assets。

真正 Actions、crates.io upload、公開下載與 Homebrew 安裝都不在離線 fake-command 測試的證明範圍內。
