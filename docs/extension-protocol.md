# 外部 extension 開發契約

CLI 版本為 package SemVer；目前0.1.0。外部協定版本為整數1，與 snapshot `reswho/9` 分開。內建與外部來源共用設定、取消、deadline、核心裁決與 rendering；extension 不直接輸出 Markdown／TUI，也不新增既有 PID metrics。

## 安裝與版本

```sh
reswho extensions install /absolute/path/manifest.json
reswho extensions --json
reswho extensions disable example
reswho extensions enable example
reswho extensions remove example
```

完整 manifest 範例是 [example.json](../examples/extensions/example.json)，原生 Rust 範例是 [extension_example.rs](../examples/extension_example.rs)。先執行 `cargo build --example extension_example`，manifest 指向本機 `target/debug/examples/extension_example`。沒有 marketplace、下載或設定編輯器。Install 只讀 metadata、驗證 binary 與解析路徑，不執行 provider。相對 executable 以 manifest 所在目錄解析並保存絕對路徑；`args` 只允許省略或空陣列，設定與輸入走 JSON stdin，不能以 interpreter＋script args 包裝入口。

外部入口只支援開發者提供的 macOS native binary；沒有 script extension、內嵌或自動安裝的 Node／Python／Lua runtime。安裝 shell script 可以下載／放置 binary＋manifest，但不能作為 extension executable。安裝與每次執行前檢查可執行權限、Mach-O 64-bit executable header、host architecture（含 universal slice）、有界 load commands 與 entry point；kernel exec 仍負責完整載入驗證。Shebang、文字、截斷、錯誤架構及非 executable Mach-O 被拒絕。既有入口遺失或被替換成 script 時 effective disabled，保留 explicit enabled、版本、順序與設定。

這是可信 package 的入口契約，不是編譯 binary 的語言來源辨識，也不是 OS sandbox。禁止交付 interpreter＋script payload；無法可靠判定合法 binary 內部是否嵌入 runtime 或呼叫其他程式。開發者須提供目標 macOS／architecture 的 binary，使用者只需放置／安裝。不防禦同使用者在驗證與 exec 之間惡意替換檔案；安裝者須保護 package 的寫入權限。

`id` 使用小寫 ASCII、數字、`_`、`-`，不可與 builtin 重複。`version` 必須是完整 canonical SemVer。`engines.reswho` 必填，使用 [node-semver 2.2.0](https://docs.rs/node-semver/2.2.0/node_semver/) 的 npm range 語意，例如 `>=0.1.0 <0.2.0`。支援 `>`／`>=`／`<`／`<=`、exact、`^`、`~`、wildcard、hyphen range、AND、`||`；不使用 Rust VersionReq，不啟用 loose 或隱含 prerelease。`^0.2.0` 不包含0.3.0；`^0.0.3` 不包含0.0.4。npm engines 通常是 advisory，reswho 則強制跳過不相容來源。

合法但 CLI range／protocol／platform 不相容的 manifest 可保存，但不注入新 definitions，也不執行。非法 JSON／SemVer／range 或不相容設定定義會拒絕整筆 install/update，保留前一狀態。清單顯示 version、requires、compatible、explicit_enabled、enabled、reason 與 unknown availability；unknown 不代表已探測到服務。

## State 與設定

State 是 selected `--config PATH` 所在目錄的 `extensions.json`；未指定時使用預設 config root。讀取指令不建立檔案。Mutations 使用 `extensions.lock` 的 flock 序列化，寫0600 temp、fsync，再 rename；corrupt／未知版本 state 不覆寫。Config 本身仍由使用者保存，安裝操作不改寫它。

同 id install 是 update，保留順序與原本明確 disabled 狀態；remove 只取消登記，保留 tombstone／原順序、使用者設定與 canonical shared definitions，重裝回原順序。停用／移除來源不使其他來源接管 canonical default／owner；不相容改動需新 key 或明確遷移。Uses 可先引用稍後安裝的 shared key；未解析前 effective disabled，後續來源補齊後可生效。

Manifest [Schema](../schemas/reswho-extension-manifest-1.schema.json) 與 `definitions` 沿用 settings 定義（provider／version 可省略，由 manifest 補入），`uses` 同樣使用攤平的 `{key, type, ...}`，但不含 default。Provider 只收到自己宣告的 definitions／uses 的有效設定值，不收到其他設定，包括其他敏感設定。CLI > user > default，設定錯誤不 clamp／fallback。Settings 與安裝 metadata 於啟動時載入；TUI 不做 hot reload 或跨輪 resource cache。

### Public settings metadata

設定定義使用 `key`、`type: "number" | "string" | "boolean"`、必填 `default`、description、unit、sensitive、provider／version。`options` 為非空、同型別 scalar 陣列，代表單選；未提供時為自由輸入。Number 可帶 min／max、`integer: true`；一般 number 支援有限負數與小數，integer 不接受分數或不安全的整數表示。Core sampling／refresh 仍是安全整數毫秒，不 clamp 或 round。所有 default／options／effective values 都須符合型別與限制。Boolean 對應 checkbox、無 options 的 number／string 對應數字／文字輸入、有 options 則是單選；本版沒有設定 editor 或 multiselect。

例如 `{"key":"shared.factor","type":"number","min":-2.5,"max":2.5,"options":[-1.5,0,2.5],"default":0,"description":"Factor","sensitive":false}`。Uses 使用同樣 type／options／min／max／integer，但沒有 default。相容性比較包含 integer 約束，options 比較集合而不覆寫原順序／default；錯誤或混型 options 整筆拒絕。`settings --json` 使用相同欄位；敏感 options 不公開，敏感 default／value 顯示遮蔽值。

## 一次呼叫

以原生 executable 啟動一次程序；stdin 是單一 JSON，寫完關閉。stdout 必須是單一 JSON response，不可混入 log；stderr 有界收取但不回顯。非零退出、crash、stderr diagnostics、timeout、過大輸出或錯誤 JSON／版本／身分只使該來源失敗。

Request 範例：

```json
{
  "protocol_version": 1,
  "host": {"version": "0.1.0", "protocol_version": 1},
  "provider": "example",
  "version": "1.0.0",
  "capability": "classify",
  "settings": {"sampling.interval_ms": 400},
  "processes": [{"pid": 42, "started_unix_us": 123, "ppid": 1, "uid": 501, "name": "demo", "executable": "/tmp/demo", "cwd": "/tmp/work"}]
}
```

`host.version` 是實際編譯的 `CARGO_PKG_VERSION`，不是 manifest 宣稱的版本。`classify` 每次批次傳本輪有已知身分的 observed metadata，不包含 environment 或未要求的 raw command。`details` 只傳 selected PID 的 `process` 物件（同上欄位）。`collect` 不傳 processes。沒有每 PID 啟動一次 classifier，也沒有持久 provider daemon。

Response 共通 envelope：

```json
{
  "protocol_version": 1,
  "provider": "example",
  "version": "1.0.0",
  "contribution": {"capability": "classify", "proposals": []}
}
```

provider／version／protocol 必須與登記及 request 相符。能力不得錯置。每輪最多4個 provider 同時執行，按登記序採納結果；所有外部能力及 runtime collection 共用 host 採樣後的10秒 deadline，逾時結果拒絕。來源本輪一旦失敗，不再呼叫其剩餘能力；下一輪可重試，不保存失敗歷史。取消／逾時會終止自有 process group、wait child 並關閉有界 pipe。

Bounds：metadata／state 各1MiB、最多128個安裝序號（含 tombstone）、每個 manifest 最多256 definitions＋uses、空 args；request／stdout／stderr 各8MiB；每批最多16384 processes／proposals，collect 最多4096 containers，details 最多64 fields。核心不把裁切的資料假裝完整；超限拒絕該來源。

每次外部呼叫增加一個內建原生 supervisor，沒有外部呼叫時不建立 supervisor；最多4個並行。Supervisor 在 fork provider 前關閉所有非必要 descriptors（含 host 與 sibling lifeline writer），只保留 stdin／stdout／stderr 與專用 FD3 reader，再以 pipe gate 先建立 provider process group、註冊 kqueue，最後允許 exec。kqueue 等待 host lifeline EOF 或 provider NOTE_EXIT，沒有忙輪詢或全域程序掃描；不再有 provider 先退出才登記 monitor 的空窗。初始化失敗時不允許 provider exec，回收後以125退出。

Host 正常退出、Ctrl-C／SIGTERM、crash／SIGKILL 或取消／deadline 關閉 lifeline 時，supervisor 終止自有 group、wait provider 並退出。Provider 正常／非零退出時也清理同群子孫；清理 group 前不 reap provider，保留 PID／PGID 身分與原始 exit code／signal。Host wait supervisor。Supervisor 提前關閉 Rust spawn 的 exec-error pipe，因此 spawn 成功只表示已建立 supervisor；provider exec 失敗以有界輸出的失敗 status 回報，不依賴原 error pipe。FD3 在 stdio 完成重導後才配置，provider 在 gate 後關閉它。

Extension 契約禁止 daemonize、setsid／setpgid 逃逸與持久 helper；保證涵蓋正常／合作 extension 及仍在管理 group 的所有子孫。刻意逃逸、外部惡意直接殺 guard、OS-wide failure 或不可中斷 kernel wait 不在保證內；這不是任意 native code containment 或 OS sandbox。

## Classify：作者自有證據

```json
{
  "capability": "classify",
  "proposals": [{
    "identity": {"pid": 42, "started_unix_us": 123},
    "evidence": {"kind": "custom", "key": "workspace", "label": "Example workspace", "path": "/tmp/work", "basis": "cwd"}
  }]
}
```

作者可自行檢查 marker、解析本機 metadata，再提出 namespaced key／label。核心要求 PID＋start 與本輪觀測一致，basis 為 cwd 或 executable，path 必須等於相應 observed absolute path。這是可追溯的 extension 證據，不等同核心已驗證作者所有推論。Key／label 各最多128 bytes。核心保留直接 app、project 與已驗證 ancestor 的優先序；只有原本 unclassified 的程序可依 custom evidence 歸類，無任意 priority，也不增加／複製 PID metrics。

snapshot attribution source 為 `extension`，`extension` 物件保存 provider／version／key／label／basis，path 沿用 attribution.path。不同 provider 的 key 不互相覆寫。

## Details：parsed typed fields

```json
{
  "capability": "parsed_details",
  "identity": {"pid": 42, "started_unix_us": 123},
  "fields": [{"key": "extensions.example.project", "label": "Project", "value": "demo", "sensitive": false}]
}
```

Field key 必須在 `extensions.<id>.*` 下且不重複；key 最多256 bytes，label 最多128 bytes，string value 最多16384 bytes。Value 只接受 JSON string／number／boolean／null，不接受任意 HTML、程式碼或巢狀物件。核心在敏感 flag 或已知秘密 key（例如 token／password／credential）時改為 `[filtered]`，MD／TUI 另做 escaping。Parsed fields 不必偽裝 argv；native command 仍使用既有受控 sanitizer。只在 selected-PID details 查詢時傳回，關閉詳情後不再要求。核心再次核對 selected process 身分；未知／重用 PID 不採納。

## Collect：未來 runtime 可用

Contribution 為 `{"capability":"collect","runtime":{...},"containers":[...]}`，沿用 [JSON DTO 契約](json-format.md) 的 Runtime／Container 結構。Runtime name 必須是 provider id，endpoint_ref 必須存在，container runtime／endpoint_ref 必須符合該 runtime；id 不可空或重複，CPU 必須非負且 finite，數值總量不得溢位。來源可回報 unknown／partial，不得提供 host PID metrics 造成重算。Diagnostics 由核心轉成無 raw payload 的來源診斷；extension 不控制核心 diagnostic operation。`--no-containers` 不呼叫 collect，但 classify／details 仍可使用。此版沒有 Podman adapter。

## 隔離範例

```sh
cargo build --example extension_example
task_dir=$(mktemp -d)
printf '%s\n' '{"version":1,"settings":{"sampling.interval_ms":400}}' > "$task_dir/config.json"
touch "$task_dir/.reswho-example"
reswho --config "$task_dir/config.json" extensions install "$PWD/examples/extensions/example.json"
reswho --config "$task_dir/config.json" extensions --json
```

在該目錄啟動自己管理的程序，再用 `--no-containers --from pid:PID --json` 或 TUI 詳情查看。完整自動化與清理見 Rust external fixtures 及 PTY runner：只有 isolated temp state/config、owned shell／provider children，不修改真正使用者 config。使用完先回收自己啟動的程序，再刪除 task_dir。Provider 具執行使用者權限，這不是 OS 沙箱；Rust unwind boundary 不攔截 panic=abort／native fault，外部程序退出則由 runner 隔離。

## Drop-in discovery（rw-31）

啟動時掃描 selected config root 的 `extensions/<id>/manifest.json`。預設 root 是 `$XDG_CONFIG_HOME/reswho`，未設 XDG 時是 `$HOME/.config/reswho`；`--config PATH` 改用 PATH 的 parent。同一次啟動不 hot reload，`--json-schema`／help 不掃描。沒有 downloader 或 runtime installer；開發者可自行提供安裝 shell script，下載目標 native binary＋manifest 後放到固定 layout。

例如安裝範例的 package 結構：

```text
<config-root>/extensions/example/
  manifest.json
  provider
```

`provider` 是編譯的 Rust example binary，manifest 使用現有 example.json 的 metadata，但 executable 改為 `provider`。資料夾名稱必須等於 manifest id；package／manifest 不接受目錄或 manifest symlink 別名，canonical executable 須在 package 內，可為 package 內 symlink。所有 executable 仍須通過 native-only 驗證。檔案與目錄上限各有界：root 最多128 entries（超限則本次不採用 drop-ins）、manifest／state 各1MiB、id／version 各64 chars；非 regular file 以 nonblocking open＋FD metadata 拒絕。Malformed／不相容／同名／builtin 名稱／缺少入口只隔離該候選，不執行它，也不阻斷健康 explicit 來源。

新批次依 id 排序追加原安裝序；更新、停用、重啟不重排。Discovery 可在 metadata 有變更時寫入 extensions.json（既有 flock＋atomic rename），不改 config.json；無變更不重寫。保存失敗或與並行 mutation 衝突時，新／變更候選 effective disabled，reason=persistence_failed；未變更且安全的既有來源可用。共用 canonical definitions 持續保留，不因資料夾消失、來源停用／移除或候選預設值改變而轉移。

State 的 origin 分為 legacy、explicit（保存 manifest path）、drop_in（保存固定 manifest path）。舊 state 缺 origin 視為 legacy explicit ownership；不自動被同名 drop-in 接管。使用者明確 install 可為 legacy 補上來源；不同 active source path 拒絕，不悄悄覆寫。相同 drop-in manifest 的 explicit install 仍保留 drop-in ownership；remove 後可明確 install 新來源。Builtins 不可 remove 或 overwrite。

Drop-in 資料夾刪除、manifest／binary 失效或壞更新時，原登記與設定保留，但本次不執行舊 metadata。有效 package 回來後沿用原序號與 enabled 狀態；明確 remove 留 tombstone，即使檔案仍在／改版也不復活，須明確 install 才恢復。Explicit／legacy 入口由其登記路徑管理，不因同名 drop-in 消失而被接管或移除。

## 本次錯誤與 profiling（rw-32）

TUI 用 `e` 開啟獨立 Extensions 頁，Esc／e 返回原頁，保留主樹選取與 scroll；開頁不要求採集。主頁有錯誤時顯示精簡 issue count＋e 提示。頁面顯示本次 provider／version／phase／result／reason／duration_ms；不將完整 diagnostic 重複塞進主樹。Markdown 一般輸出顯示每來源的精簡錯誤；`reswho --md --profile` 加計時表。JSON snapshot 升為 `reswho/10`，新增 extensions report，保留 Schema8／9。`extensions --json` 保留 metadata array，新增各 entry 的 report（drop-in read／validate／register，以及能力 never_invoked／skipped）；全 registry read scope 同時保留在 snapshot report。離線清單不呼叫 collect／classify／details、不探測服務。

每輪重新建立 report，startup read／validate／register 資料與本輪實際能力分開 phase。只用 Instant 量測實際執行，duration_ms 為可含小數的非負數；skipped／never_invoked 為 null，0 不代表未呼叫。失敗嘗試也保留 duration；核心拒絕 contribution 時標記失敗，不能只以 provider return 當成功。Native command details 及 external details 都只沿用真實按需查詢，不為 profiling 多跑一次。collection_ms 是整輪 wall time；startup、共享批次驗證及平行 provider durations 有重疊，不可加總當總耗時。Worker 失敗時 TUI 保留本次 report，舊 snapshot 僅作 stale 資源畫面。

Report 最多2048 records，provider／version 最多64 chars，reason 最多128 chars，使用固定安全 reason code，不回顯 provider Err／stderr／panic payload、原始 manifest 或設定值。只保留本次結果；不寫 logfile、歷史資料或跨輪 resource cache，也不建立 daemon。
