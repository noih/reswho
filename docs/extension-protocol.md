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

完整 manifest 範例是 [example.json](../examples/extensions/example.json)，執行程式是 [example.py](../examples/extensions/example.py)。沒有 marketplace、下載或設定編輯器。Install 只讀取 metadata 與解析 executable 路徑，不執行 provider。相對 executable 以 manifest 所在目錄解析並保存絕對路徑；args 原樣傳給 executable，不經 shell 展開。

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

以 executable＋args 啟動一次程序；stdin 是單一 JSON，寫完關閉。stdout 必須是單一 JSON response，不可混入 log；stderr 有界收取但不回顯。非零退出、crash、stderr diagnostics、timeout、過大輸出或錯誤 JSON／版本／身分只使該來源失敗。

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

Bounds：metadata／state 各1MiB、最多128個安裝序號（含 tombstone）、每個 manifest 最多256 definitions＋uses、64 args；request／stdout／stderr 各8MiB；每批最多16384 processes／proposals，collect 最多4096 containers，details 最多64 fields。核心不把裁切的資料假裝完整；超限拒絕該來源。

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
task_dir=$(mktemp -d)
printf '%s\n' '{"version":1,"settings":{"sampling.interval_ms":400}}' > "$task_dir/config.json"
touch "$task_dir/.reswho-example"
reswho --config "$task_dir/config.json" extensions install "$PWD/examples/extensions/example.json"
reswho --config "$task_dir/config.json" extensions --json
```

在該目錄啟動自己管理的程序，再用 `--no-containers --from pid:PID --json` 或 TUI 詳情查看。完整自動化與清理見 Rust external fixtures 及 PTY runner：只有 isolated temp state/config、owned shell／provider children，不修改真正使用者 config。使用完先回收自己啟動的程序，再刪除 task_dir。Provider 具執行使用者權限，這不是 OS 沙箱；Rust unwind boundary 不攔截 panic=abort／native fault，外部程序退出則由 runner 隔離。
