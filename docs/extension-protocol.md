# 外部 extension 開發契約

CLI 版本為 package SemVer；目前0.1.0。外部協定版本為整數1，與 snapshot `reswho/11` 分開。內建與外部來源共用設定、取消、deadline、核心裁決與 rendering；extension 不直接輸出 Markdown／TUI，也不新增既有 PID metrics。

## 安裝與版本

```sh
reswho extensions install /absolute/path/manifest.json
reswho extensions --json
reswho extensions disable example
reswho extensions enable example
reswho extensions remove example
```

完整 manifest 範例是 [example.json](../examples/extensions/example.json)，原生 Rust 範例是 [extension_example.rs](../examples/extension_example.rs)。先執行 `cargo build --example extension_example`，manifest 指向本機 `target/debug/examples/extension_example`。沒有 marketplace 或下載器；TUI 主畫面的 `s` 可開啟設定草稿 popup。Install 只讀 metadata、驗證 binary 與解析路徑，不執行 provider。相對 executable 以 manifest 所在目錄解析並保存絕對路徑；`args` 只允許省略或空陣列，設定與輸入走 JSON stdin，不能以 interpreter＋script args 包裝入口。

外部入口只支援開發者提供的 macOS native binary；沒有 script extension、內嵌或自動安裝的 Node／Python／Lua runtime。安裝 shell script 可以下載／放置 binary＋manifest，但不能作為 extension executable。安裝與每次執行前檢查可執行權限、Mach-O 64-bit executable header、host architecture（含 universal slice）、有界 load commands 與 entry point；kernel exec 仍負責完整載入驗證。Shebang、文字、截斷、錯誤架構及非 executable Mach-O 被拒絕。既有入口遺失或被替換成 script 時 effective disabled，保留 explicit enabled、版本、順序與設定。

這是可信 package 的入口契約，不是編譯 binary 的語言來源辨識，也不是 OS sandbox。禁止交付 interpreter＋script payload；無法可靠判定合法 binary 內部是否嵌入 runtime 或呼叫其他程式。開發者須提供目標 macOS／architecture 的 binary，使用者只需放置／安裝。不防禦同使用者在驗證與 exec 之間惡意替換檔案；安裝者須保護 package 的寫入權限。

`id` 使用小寫 ASCII、數字、`_`、`-`，不可與 builtin 重複。`version` 必須是完整 canonical SemVer。`engines.reswho` 必填，使用 [node-semver 2.2.0](https://docs.rs/node-semver/2.2.0/node_semver/) 的 npm range 語意，例如 `>=0.1.0 <0.2.0`。支援 `>`／`>=`／`<`／`<=`、exact、`^`、`~`、wildcard、hyphen range、AND、`||`；不使用 Rust VersionReq，不啟用 loose 或隱含 prerelease。`^0.2.0` 不包含0.3.0；`^0.0.3` 不包含0.0.4。npm engines 通常是 advisory，reswho 則強制跳過不相容來源。

合法但 CLI range／protocol／platform 不相容的 manifest 可保存，但不注入新 definitions，也不執行。非法 JSON／SemVer／range 或不相容設定定義會拒絕整筆 install/update，保留前一狀態。清單顯示 version、requires、compatible、explicit_enabled、enabled、reason 與 unknown availability；unknown 不代表已探測到服務。

## State 與設定

State 是 selected `--config PATH` 所在目錄的 `extensions.json`；未指定時使用預設 config root。讀取指令不建立檔案。Mutations 使用 `extensions.lock` 的 flock 序列化，寫0600 temp、fsync，再 rename；corrupt／未知版本 state 不覆寫。安裝操作不改寫 config；TUI 設定 popup 以既有原子保存路徑寫入使用者確認的草稿。

同 id install 是 update，保留順序與原本明確 disabled 狀態；remove 只取消登記，保留 tombstone／原順序、使用者設定與 canonical shared definitions，重裝回原順序。停用／移除來源不使其他來源接管 canonical default／owner；不相容改動需新 key 或明確遷移。Uses 可先引用稍後安裝的 shared key；未解析前 effective disabled，後續來源補齊後可生效。

Manifest [Schema](../schemas/reswho-extension-manifest-1.schema.json) 與 `definitions` 沿用 settings 定義（provider／version 可省略，由 manifest 補入），`uses` 同樣使用攤平的 `{key, type, ...}`，但不含 default。Provider 只收到自己宣告的 definitions／uses 的有效設定值，不收到其他設定，包括其他敏感設定。CLI > user > default，設定錯誤不 clamp／fallback。Settings 與安裝 metadata 於啟動時載入；TUI 不做 hot reload 或跨輪 resource cache。

### Public settings metadata

設定定義使用 `key`、`type: "number" | "string" | "boolean"`、必填 `default`、description、unit、sensitive、provider／version。`options` 為非空、同型別 scalar 陣列，代表單選；未提供時為自由輸入。Number 可帶 min／max、`integer: true`；一般 number 支援有限負數與小數，integer 不接受分數或不安全的整數表示。Core sampling／refresh 仍是安全整數毫秒，不 clamp 或 round。所有 default／options／effective values 都須符合型別與限制。Boolean 對應 checkbox、無 options 的 number／string 對應數字／文字輸入、有 options 則是單選；TUI 設定 popup 支援草稿編輯、驗證、取消與保存；沒有 multiselect。

例如 `{"key":"shared.factor","type":"number","min":-2.5,"max":2.5,"options":[-1.5,0,2.5],"default":0,"description":"Factor","sensitive":false}`。Uses 使用同樣 type／options／min／max／integer，但沒有 default。相容性比較包含 integer 約束，options 比較集合而不覆寫原順序／default；錯誤或混型 options 整筆拒絕。`settings --json` 使用相同欄位；敏感 options 不公開，敏感 default／value 顯示遮蔽值。

## 穩定性與能力協商

正式外部 DTO 位於 `extensions::protocol`，不引用 snapshot 的 Runtime／Container／DetailField；容器、分類與 details 使用明確 adapter 轉成內部模型，再經核心驗證。內建 Docker／Apple 保留內部 adapter。CLI／host package SemVer、extension package SemVer、外部 protocol major 與 snapshot schema 是四個獨立版本。Protocol1 尚未發版，本次直接整理契約，不提供假想舊版 adapter。

同 major 可新增「缺少時仍維持原意」的可選 metadata。Envelope、observed process、resource、metric、classification evidence、parsed field 與 manifest 根物件忽略未知 metadata 欄位；未知欄位不會被轉成核心行為。Settings definitions／uses 的 constraint 欄位仍嚴格驗證，未知 constraint 拒絕；本機 config 與安裝 state 的版本／未知欄位規則維持原契約，不能以 wire 的寬鬆規則套用。必要欄位／型別／單位意義／既有語意的改動都是 breaking，須新 protocol major。新 unit identifier 本身可新增，但不能改變已使用 unit 的意義。

Manifest 可宣告 `required_features`（省略為空；最多64項），request 的 `host.features` 列出 host 理解的 `roles`、`resources`、`collect_scopes`。未知必要 feature 在執行前使來源不相容，不注入 definitions。Response 同樣可宣告 `required_features`；未知值拒絕整個 response。未宣告的額外 metadata 不能帶必要的 privacy／validation semantics；新必要限制須 feature gate，或新 major。遮蔽、PID identity、namespace、bounds 與 native-only 限制始終強制，不是 optional feature。

Manifest 的未知 capability 保留原始字串於 state，來源以 `unsupported_capability` 停用；不可降級後執行剩餘能力。離線清單的 `declared_capabilities` 保留完整宣告，`capabilities` 是 host 已知的投影，另列 `collect_scopes`／`required_features`。未來 host 可重新理解原始宣告。Response 未知或錯置 capability 一律拒絕。

Schema：[manifest](../schemas/reswho-extension-manifest-1.schema.json)、[request](../schemas/reswho-extension-request-1.schema.json)、[response](../schemas/reswho-extension-response-1.schema.json)。它們獨立於 snapshot Schema，涵蓋結構與公開 bounds；PID／觀測路徑、重複 identity、scope 授權、有限數字與總量溢位仍由 production parser／核心驗證。固定 fixtures 位於 `tests/fixtures/extensions/`，同時驗證 parser 與 Schema。

## 一次呼叫

以原生 executable 啟動一次程序；stdin 是單一 JSON，寫完關閉。stdout 必須是單一 JSON response，不可混入 log；stderr 有界收取但不回顯。非零退出、crash、stderr diagnostics、timeout、過大輸出或錯誤 JSON／版本／身分只使該來源失敗。

Request 範例：

```json
{
  "protocol_version": 1,
  "host": {"version": "0.1.0", "protocol_version": 1, "features": ["roles", "resources", "collect_scopes"]},
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

Bounds：metadata／state 各1MiB、最多128個安裝序號（含 tombstone）、每個 manifest 最多256 definitions＋uses、空 args；request／stdout／stderr 各8MiB；每批最多16384 processes／proposals，collect 每 scope 最多4096 containers／resources，每 resource 最多64 metrics，details 最多64 fields。核心不把裁切的資料假裝完整；超限拒絕該來源。

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

補充角色使用同樣 evidence 欄位但 `kind: "role"`。它可附在已有 app／project／ancestor／fallback 歸屬的程序上，snapshot `process.roles` 保存 provider／version／key／label／basis／path。角色只附於該 PID，不傳給子程序、不改樹、不建立 subgroup、不改 owner 或 metrics。每 PID 最多64個角色（跨來源總計）。按登記序預檢整個來源；任何 PID 超限時，拒絕該來源整份 classify contribution、記錄安全失敗並抑制本輪剩餘能力，保留先前健康來源；不靜默裁切。相同 PID 的 `(provider,key)` 不可重複；外部重複 role 拒絕整個 response。不同 provider 的同名 key 並存，依原登記序顯示，不以任意 plugin priority 裁決。MD 在原程序旁顯示 role provenance，TUI 列與 process details 可見，JSON 保留原始結構。Selected PID 再取得 details 時保留本輪 roles。

Primary fallback 的 snapshot attribution source 為 `extension`，`extension` 物件保存 provider／version／key／label／basis，path 沿用 attribution.path。不同 provider 的 key 不互相覆寫。

## Details：parsed typed fields

```json
{
  "capability": "parsed_details",
  "identity": {"pid": 42, "started_unix_us": 123},
  "fields": [{"key": "extensions.example.project", "label": "Project", "value": "demo", "sensitive": false}]
}
```

Field key 必須在 `extensions.<id>.*` 下且不重複；key 最多256 bytes，label 最多128 bytes，string value 最多16384 bytes。Value 只接受 JSON string／number／boolean／null，不接受任意 HTML、程式碼或巢狀物件。核心在敏感 flag 或已知秘密 key（例如 token／password／credential）時改為 `[filtered]`，MD／TUI 另做 escaping。Parsed fields 不必偽裝 argv；native command 仍使用既有受控 sanitizer。只在 selected-PID details 查詢時傳回，關閉詳情後不再要求。核心再次核對 selected process 身分；未知／重用 PID 不採納。

## Collect：容器與一般資源

具有 collect capability 的 manifest 必須宣告非空 `collect_scopes`，支援 `containers`／`resources`，不可重複；不提供隱含預設。未知 scope 保存但使來源以 `unsupported_collect_scope` 停用。Request 的 `collect_scopes` 是本次允許的 scopes。`--no-containers` 不執行 Docker／Apple 或容器專用 external collector；雙 scope 來源只收到 `["resources"]`。classify／details 不受該 flag 影響。

Response contribution 可同時回傳兩種資料：

```json
{
  "capability": "collect",
  "resources": [{
    "id": "disk0", "kind": "storage.volume", "label": "Data volume", "path": "/Volumes/Data",
    "metrics": [
      {"key": "capacity", "label": "Capacity", "unit": "bytes", "value": 9007199254740993},
      {"key": "temperature", "label": "Temperature", "unit": "vendor.celsius", "value": -2.5},
      {"key": "used", "label": "Used", "unit": "bytes", "value": null}
    ]
  }]
}
```

容器 scope 使用 wire `runtime`＋`containers`，兩者須一起存在；runtime name 是 provider id，endpoint_ref 必須存在，container runtime／endpoint_ref 符合該 runtime。CPU 非負有限，inactive metrics 不得存在，identity／總量檢查維持既有行為。Container wire DTO 在 protocol 模組獨立定義，不以 snapshot serializer 作為外部契約。協定整數欄位接受數值完全等值的 decimal／exponent 表示（如 `1000.0`／`1e3`）；以十進位精確解析並依欄位寬度檢查，不經 f64，拒絕分數、unsigned 負值及 overflow。沒有被請求的 scope 即使只回傳空陣列，也拒絕整個 response；不悄悄丟棄。至少提供一種已請求 scope；空 inventory 可用空陣列。

一般 resource identity 是 `(provider,id)`，provider／version 由 host 登記資料提供，不能由 payload 冒用。`id` 最多256 bytes，`kind` 最多64 bytes，metric key 最多128 bytes，unit 最多64 bytes；identifier 為 ASCII 字母、數字、`_ - . / %`。Label 非空最多128 bytes，path 可省略／null，存在時須 absolute、最多16384 bytes。不重複 resource id 或同一 resource 的 metric key；每 scope 最多4096筆，每 resource 最多64 metrics。數量超限拒絕來源，不裁切成完整資料。

Metric value 必填，為有限 JSON number 或 null；保留大整數與小數表示，不強制轉成 f64 存放，不限制負數或 percent≤100。`null` 是已觀測但未知；整個 metric 省略代表未觀測。Common units 為 `bytes`、`count`、`percent`、`seconds`；opaque／custom units 顯示原字串，不自動換算、不依 unit 推測 dimension。既有 unit 的意思不可更改。這不是 metric registry、時間序列或 UI scripting API。

來源 collect 失敗時只影響本次被請求的 declared scopes：resources-only 失敗仍保留失敗 report，不建立假的 container runtime；雙 scope 來源的 container scope 失敗會保留 unavailable inventory。

資源置於獨立 `group:resources`，provider／resource stable reference 以 provider＋id 生成。資源 category／provider 的 `grouping: "resource"` 保存獨立 scope，空內容與 depth0 投影也不假造 CPU／RAM／process totals。MD／TUI 列顯示前兩個 typed metrics，selected resource 的 MD 與 TUI `d` 顯示全部 metrics／path／provenance，JSON 保留精確 number。`--from`／depth／name sort 沿用一般樹投影；CPU／RAM／uptime sort 將一般資源視為不適用，依 stable ref 排列。Resource 沒有假造 process／CPU／RAM，絕不把 storage bytes 當 RAM，也不加進 host、container 或其他 resource 總量，即使 unit 相同亦不相加。所有文字經 renderer escaping。

Native example 回傳標記目錄的 primary fallback＋supplemental role，以及一個合成 storage volume；沒有新增 hardware collector。Provider diagnostics 仍由核心轉成安全固定 code；stderr、parser error payload、設定值不直接顯示。

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

TUI 用 `e` 開啟獨立 Extensions 頁，Esc 返回原頁（e 不切換頁面），保留主樹選取與 scroll；開頁不要求採集。主頁有錯誤時顯示精簡 issue count＋e 提示。頁面顯示本次 provider／version／phase／result／reason／duration_ms；不將完整 diagnostic 重複塞進主樹。Markdown 一般輸出顯示每來源的精簡錯誤；`reswho --md --profile` 加計時表。JSON snapshot `reswho/11` 保留 extensions report 並新增 roles／resources；Schema8／9／10 保留。`extensions --json` 保留 metadata array，新增各 entry 的 report（drop-in read／validate／register，以及能力 never_invoked／skipped）；全 registry read scope 同時保留在 snapshot report。離線清單不呼叫 collect／classify／details、不探測服務。

每輪重新建立 report，startup read／validate／register 資料與本輪實際能力分開 phase。只用 Instant 量測實際執行，duration_ms 為可含小數的非負數；skipped／never_invoked 為 null，0 不代表未呼叫。失敗嘗試也保留 duration；核心拒絕 contribution 時標記失敗，不能只以 provider return 當成功。Native command details 及 external details 都只沿用真實按需查詢，不為 profiling 多跑一次。collection_ms 是整輪 wall time；startup、共享批次驗證及平行 provider durations 有重疊，不可加總當總耗時。Worker 失敗時 TUI 保留本次 report，舊 snapshot 僅作 stale 資源畫面。

Report 最多2048 records，provider／version 最多64 chars，reason 最多128 chars，使用固定安全 reason code，不回顯 provider Err／stderr／panic payload、原始 manifest 或設定值。只保留本次結果；不寫 logfile、歷史資料或跨輪 resource cache，也不建立 daemon。
