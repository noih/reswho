# macOS 採集驗證

Tenqo：`rw-1`、`rw-2`、`rw-3`。日期：2026-11-01。環境：macOS 26.5.2、Apple Silicon、10 logical CPUs、32 GiB RAM，Rust 1.99.1。

## 可執行範圍

```sh
cargo run -- --md
cargo run -- --json --depth 1
cargo run -- --md --from group:development --depth 2
cargo run -- --md --from pid:1234 --depth all
cargo run -- --help-format
```

`pid:1234` 是範例，須替換為當前 PID。`group:` ref 可直接從輸出複製。

原型提供程序清單、兩次 CPU counter 差值、RSS、UID、PPID、啟動時間、執行時間、執行檔與工作目錄，TCP／UDP socket 端點與 TCP 狀態，以及整機 CPU／RAM／swap。沒有 daemon 或跨次執行的觀測資料庫。

支援一次性 Markdown／JSON、`--from` 與 `--depth`。非互動執行預設 Markdown；互動環境預設 TUI 的分支尚未實作，因此須明確帶 `--md` 或 `--json`。`--tui` 目前回報未實作，不會以靜態輸出冒充 TUI。

未實作：Ratatui 畫面、容器內部資料、命令參數、Review 規則與持續觀測。不根據本次取樣產生遺留判定。

## 分類與計數

- 分類不使用 app／工具名稱清單，也不假設安裝於 `/Applications`。
- 執行檔路徑符合最外層 `.app/Contents/` 或 `.app.bundle/Contents/` 結構時歸入應用，涵蓋巢狀 helper 與 clone bundle；不同完整路徑保持獨立，不按同名合併。這是路徑證據，不是簽章驗證。
- 其他程序由 cwd 向上查找 `.git` directory 或 worktree marker file；各 checkout 分組獨立。若應用另有 cwd 專案線索，保留為 candidate，主分類仍為應用。
- 系統執行檔依系統目錄結構分類；其餘保留 `Unclassified`。目前未支援非 Git 專案根目錄辨識。
- 無直接歸屬時，可沿父程序尋找應用／專案，但每一步須有相同的已知 UID、相同的已知 cwd，且父程序啟動時間不晚於子程序。不同或未知 cwd、PID 1、身分不全、循環與超過 128 層均中止推論。直接 app／專案線索不被祖先覆蓋。
- 推論在程序 evidence 明示來源 PID 與條件；可讀 PPID 不等於已確認同專案。保守規則可能漏分，不為降低未知數而硬猜。
- 移除 Docker 名稱特例，runtime adapter 尚未提供 metadata 前，依一般應用／系統路徑分類，不能視為完整容器用量。
- 同一程序只屬一個分類。不同分類的 PPID 保留，但不把程序複製到另一組。
- 同一組內的父子關係須有相容的啟動時間；循環或超過 128 層的異常鏈轉為明示的獨立根，避免遺失或重複程序。
- group ID 使用分類與來源 key 的 SHA-256 前 16 hex；同一快照會檢查碰撞並拒絕混組。識別規則屬原型契約，未來若改變必須更新格式版本。
- 深度從選取起點 0 計算，預設 2；metadata 不算資源階層。限制深度不改變已計算的 totals。
- Markdown 收合標記為原列尾端的 `▸`，不另列展開指令；用法見 `--help-format`。此標記僅影響顯示，ref 與 JSON omitted_children 不變。
- 路徑證據簡寫為 `app PATH`／`git PATH`，意義集中在 `--help-format`，不逐項描述辨識步驟。

## 指標與邊界

程序 CPU 使用 native task ticks／同一程序兩次觀測的 Mach ticks 差值，100% 代表一個 logical core。整機 CPU 使用 host ticks，100% 代表整機。程序身分在觀測前後核對 start time，缺失或不同身分不計算 CPU。

RSS 為 `proc_taskinfo.pti_resident_size`，不含被換出或壓縮部分。群組總和仍可能重複包含不同程序共用的頁，不能當成唯一實體用量。JSON totals 的 `known_*` 只加總可取得的值，並附 unknown counts；Markdown 用 `>=` 表示部分總和。

整機 RAM used 以 `(internal - purgeable + wired + compressor) × page size` 計算，與 RSS 群組總和不同。Mach VM statistics 只要求本次使用的欄位已由 kernel 回傳，不能要求整個最新版 struct 長度，否則不同 ABI revision 會被誤認為不可讀。

FFI buffer 使用 libc／libproc 的固定 C struct，以實際回傳長度驗證欄位可用性；字串由有界陣列讀取。Mach host port 在查詢後釋放。不讀取環境變數或命令參數。Markdown 路徑／名稱轉義控制字元、換行、方向控制字元與結構符號。

參考：[Apple proc_info.h](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/sys/proc_info.h)、[sysinfo macOS counter 實作](https://docs.rs/sysinfo/0.40.1/src/sysinfo/unix/apple/macos/process.rs.html)。計數器直接使用 libc／mach2，socket struct 使用 libproc，沒有 sysinfo 相依。

Markdown 在群組 RAM 後顯示 `oldest`：組內最長的已知 uptime，包含閒置時間；未知成員存在時加 `>=`，全部未知為 `?`。數值在限制深度前計算，收合不影響結果。JSON `reswho/2` 的 totals 加入 `oldest_uptime_seconds` 與 `uptime_unknown`，既有欄位保留。

目前 Markdown 為 `reswho-md/4`。一般樹狀展開只顯示程序摘要；`--from pid:N` 顯示選取 PID 的詳細資訊，其子程序仍為摘要。完全無可讀身分、路徑或指標的程序合併成 `Unreadable · N processes`，不附 ref／展開箭頭；部分可讀程序不折疊。此為 Markdown 呈現規則，不改變分類與 totals；JSON 保留個別程序，明確 PID 查詢仍可讀取診斷原因。

## Socket 採集與格式

透過 `proc_pidinfo(PROC_PIDLISTFDS)` 與 `proc_pidfdinfo` 讀取 TCP／UDP IPv4／IPv6 socket，使用 libproc 的 C struct，核對完整回傳長度後才解碼。FD inventory 最多重試 3 次、上限 65,536 筆；超過範圍回報不可用。libproc 的建置需要 macOS SDK／clang。

同一程序內依 OS socket identity 去除重複 descriptor，不跨程序加總連線數。對外只呈現 FD、protocol、local／remote endpoint、TCP state，不輸出 kernel pointer。FD 消失、權限或 ABI／IP metadata 不完整時保留部分結果及錯誤。整個程序身分在採集前後核對；變更或不可確認時丟棄觀測，保留原始 OS 錯誤。

- `network: null`：無法取得可信的 socket inventory。
- `network.complete: false`：僅取得部分已列出的 descriptor，附 `unavailable` 原因；`true` 只代表本次列出的 FD 查詢完成，非原子快照或活動歷史。
- Markdown ports：`tcp:8080` 為監聽，`udp:5353` 為 bound UDP port（不代表服務端）。已建立 TCP 連線另列 endpoint／state。
- `-` 表示本次成功採集範圍內沒有符合項目，`?` 表示未知，`tcp:8080 + ?` 表示已知項目加未知部分。零連線不表示可以停止。
- 尚未收集 Unix socket、流量或無活動時長。

rw-2 當時格式更新為 `reswho-md/2`／`reswho/2`（目前 Markdown 為 `/4`）：新增 network 資料與逐程序歸屬證據；移除名稱特例，因此舊 `group:containers`／Unknown project 分組可能不存在。hash 演算法不變，沒有提供舊 group ref alias；應從新快照複製 ref。

參考：[libproc socket types](https://docs.rs/libproc/0.14.11/libproc/net_info/index.html)、[Apple proc_info.h](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/sys/proc_info.h)。

## rw-3 摘要與詳情驗證

`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、21 項一般測試與受影響的 process live test 通過。新增檢查覆蓋摘要不帶逐程序 metadata、零用量／部分可讀程序保留、Unreadable 合併計數與無展開標記、明確 PID 診斷，以及 JSON／總計不變。原生 socket 採集未修改，沿用既有真機證據。

真機 `--md --depth 3` 將 188 個無可讀資料的程序合併成一列；一般程序列沒有 PPID／UID、路徑、socket 詳情或逐項權限錯誤。既有 live fixture 的 sleep 與暫存目錄已回收。此輪沒有實作或執行管理員提權。

## rw-2 真機證據

一般使用者、工具沙箱外、未使用 sudo，單次最終採集：780 程序，Applications 233、System 178、Development 107、Unclassified 262；其中 189 個程序身分無法確認。591 個程序完成 socket 採集、0 個部分結果，共觀測 182 個 TCP／UDP socket（此為採集記錄數，不是跨程序去重後的系統連線總數）。17 個程序使用明示的祖先推論。

採集 275ms（含約 250ms 等待），完整執行約 290ms，0.03s user／0.01s sys，最大 RSS 10,010,624 bytes（約 9.5 MiB）；完整 JSON 約 1.18 MB。加入網路資訊後仍屬單次量測，非效能保證。

此輪修改前的工具沙箱內基線為 786 程序、Unclassified 257、身分未知 189、採集 281ms；中途新版採集 792 程序、Unclassified 266、身分未知 193。系統程序持續變動，不能當成同一資料集的分類正確率比較；移除名稱猜測也可能增加未知。中途採集的身分失敗中，192 個為 `Operation not permitted (os error 1)`，1 個為 `No such process (os error 3)`，現在會保留區別。

工具沙箱限制 `/usr/bin/time -l` 的 `sysctl kern.clockrate`；上述最終成本使用沙箱外量測。Markdown 另以自有 loopback TCP／UDP socket 驗證，顯示對應監聽埠與端點，退出即關閉。

## rw-1 歷史基線

以下為 commit `613e0a1` 的舊版規則與測量，不代表目前分類。

最終原型以一般使用者權限執行（工具沙箱外，沒有 sudo）：

| 指標 | 結果 |
| --- | --- |
| 程序數 | 868 |
| cwd 可讀 | 634 |
| executable 可讀 | 629 |
| CPU／RSS 可讀 | 各 634 |
| 分類 | Applications 188、System 206、Development 144、Container hosts 27、Unclassified 303 |
| 採集時間 | 268ms，包含約 250ms 等待；完整執行約 296ms |
| `/usr/bin/time -l` | 0.03s user、0.00s sys；最大 RSS 7,733,248 bytes（約 7.4 MiB） |

這是單次最終 smoke 的結果，並非效能保證。較早版本三次有界取樣得到約 295–298ms，僅作開發成本參考；數值隨系統負載與程序數改變。使用 `--depth all` 時輸出量約 1 MiB，日常應使用預設深度與分支查詢。

在工具沙箱內仍可取得主要程序資訊，但 swap query 被限制，呈現 `?`／`null` 並附警告；解除沙箱後可取得 swap。兩種環境都仍有其他使用者／受保護程序不可讀，不以零補值。

容器唯讀盤點：

- Docker client／server 29.9.2，context `desktop-linux`，`docker ps` 可用。
- Apple container CLI 1.1.0，`container list --format json` 回報 `XPC connection error: Connection invalid`，沒有自動啟動服務。
- PATH 未發現 Podman。未驗證其 adapter，也不宣稱支援完成。

## 驗證與保留資源

- `cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`。
- `cargo test`：19 項一般測試通過；2 項 live test 預設 ignored。新增規則先重現名稱誤判與 clone bundle 漏分，再完成修正。
- `cargo test --test live -- --ignored --test-threads=1`：2 項真機測試通過，驗證 PID／PPID、cwd、CPU／RSS、專案歸類、group 查詢、預設非互動 Markdown、無 socket，以及 TCP listener／Established、UDP、IPv4／IPv6、重複 FD 去重。IPv6 dual-stack 解碼調整後針對 socket live test 重驗通過。
- 不存在 PID：exit 1、stdout 空白、stderr 說明 target not found。
- cargo-audit 未安裝，未執行 advisory audit。
- Live fixture 的 sleep 已 kill／wait；臨時目錄由 TempDir 清除。Loopback sockets 隨 handle／測試程序退出釋放。沒有啟動容器或常駐採集程序。
- `target/` 約 636 MiB，為 reswho 後續開發保留的共用 build cache；下一階段邊界檢查大小，不與其他工作共用或任意清理其他 cache。

下一階段先補容器 adapter，再基於已驗證的資料接上 Ratatui 和 Review。
