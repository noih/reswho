# JSON 資料契約

目前版本：`reswho/12`。規格檔：[reswho-12.schema.json](../schemas/reswho-12.schema.json)，使用 JSON Schema Draft 2020-12。它是 reswho 的資料契約，並非跨監控工具共用的資料模型。

```sh
reswho --json --depth all > snapshot.json
reswho --json --from pid:1234 --depth 0
reswho --json-schema > reswho.schema.json
```

`--json-schema` 不採集程序、不查詢 Docker、不連網。Schema 僅使用內部 `$ref`，可離線驗證。Schema 的 `$id` 是 `urn:reswho:snapshot:12`，不是下載網址。Schema 使用 [Draft 2020-12](https://json-schema.org/draft/2020-12)；測試以 [jsonschema](https://docs.rs/jsonschema/0.60.2/jsonschema/) 驗證。

## 輸出與版本規則

- `--json` 成功時 stdout 為一個 UTF-8 JSON object，加結尾換行；不混入進度、ANSI 控制碼或 Markdown。欄位順序與縮排沒有語意。
- 採集到部分資料仍可成功輸出；必須檢查診斷、unknown 計數與 `inventory_complete`，不能把 exit 0 解讀為所有資料完整。
- 參數錯誤、查無指定 ref、無法建立快照或序列化失敗：非零 exit、診斷寫 stderr，不輸出半份快照。stdout 寫入途中發生 I/O 錯誤可能留下部分資料；接收端仍需完成 JSON parsing。下游關閉 pipe 時視為正常結束。
- 預設 `--depth 2`；需要完整觀測樹時明確使用 `--depth all`。完整樹不表示所有系統資訊均可讀取。
- `--sort cpu|memory|uptime|name` 控制各層 children 排序，預設 memory。數值降序、未知／不適用置後，名稱自然升序；同值以 ref 排序。程序用自身數值，群組用範圍合計（部分合計用已知下界），不改歸屬或 totals。陣列順序仍不是識別契約。
- 先檢查 `schema`。新版本可能只輸出最新契約，不保證能指定舊版。
- 刪除／改名欄位、改型別、改單位、改 null 或統計語意、改 ref 生成方式、增加封閉 enum 值，都須增加 `reswho/N` 的 N 並提供遷移說明。
- 新增可忽略的欄位可視為相容擴充；讀取者必須忽略未知 object 欄位。Schema 的 `additionalProperties` 因此允許擴充。既有必填欄位不能消失，不能把新欄位變成舊讀取者的必需資訊。
- `kind`、`grouping`、attribution source、diagnostic code、runtime status、socket protocol、project source 為 Schema 定義的封閉 enum。container state、TCP state、port protocol、memory basis 等來源描述字串不是封閉 enum，讀取者須保留未知值。
- `name`、`evidence`、診斷 `message` 是展示文字，可以改措辭，不作解析或分支條件。兄弟節點、診斷與 evidence 的順序不是契約，不得用陣列索引當識別碼。
- Markdown／TUI 排版變更不改 JSON。程式處理請使用欄位，不解析 `Σ`、時間符號或文字單位。

## 指定 PID 的命令資訊

`--json --from pid:N` 只查詢選取程序的命令，子程序與一般群組查詢不查詢；TUI 只有開啟程序詳情時才查詢選取 PID。新增的 `process.command` 是 `/8` 相容擴充；新版輸出此欄，舊版輸出可缺省。`null` 表示未要求查詢；物件存在但 `argv: null` 表示已要求、但無法取得，原因在該物件的 `unavailable`。

| 欄位 | 意義 |
| --- | --- |
| `argv` | 保留順序與空字串的參數陣列，包含 argv[0]；值可能已遮罩。不是可直接執行的 shell 字串 |
| `redacted_indices` | 被全部或部分遮罩的 argv 索引（從 0 起算），不依 `[filtered]` 字面值判斷 |
| `entrypoint` | 明確啟動形式中的 script／JAR 參數，否則 null。它不是實際開啟檔案或專案歸屬的證明 |
| `unavailable` | 命令採集的 Diagnostic 陣列；成功為空 |

例如 `python3 worker.py --token VALUE` 會保留參數位置、將 VALUE 替換為 `[filtered]`。敏感選項值、assignment 的值、URL、header、控制字元與常見 inline-code 形式會遮罩。短選項無法跨所有工具統一解讀，採保守遮罩，可能隱藏非敏感值。這是已知形式的遮罩，不是完整秘密偵測；未識別的自訂位置參數仍可能含敏感資料。沒有輸出未遮罩命令的選項。

敏感名稱涵蓋 token、pwd、password/passwd、passphrase/passkey、auth/authorization、secret、credential、api-key/access-key/private-key、cookie、header、session、bearer，以及 key/pass/user/env。長參數名稱忽略大小寫及分隔符，涵蓋 `--api-key`／`--api_key`／`--apiKey`、`--db-pwd` 等變體；`--name VALUE` 與 `--name=VALUE` 都會遮罩值。所有 assignment 的值仍採保守遮罩，即使名稱不在清單中。參數名稱保留，例如 `-token VALUE` → `-token [filtered]`、`-password=VALUE` → `-password=[filtered]`、`-pVALUE` → `-p[filtered]`；不刪除參數或改變其順序。

入口只處理 node/nodejs、Python（含數字版本）、Ruby、Perl、sh/bash/zsh 的直接 script，以及 `java -jar FILE`。Python 可略過已知無值啟動選項；未知選項、module/eval、wrapper 與複雜啟動形式不猜入口。相對參數保留原樣：目前 cwd 不一定是啟動時 cwd，因此不拼接絕對路徑，也不改變分類。

原生介面為 macOS `KERN_PROCARGS2`。只解析 argc 指定的參數，尾端可能帶有的環境資料不解析、不保存、不輸出。限制為 1 MiB buffer 加 argc header、4096 個參數、每參數 16 KiB；超限、buffer 填滿而完整性不明、非 UTF-8 或截斷會整份回報 unavailable。依目標程序位元寬度計算 executable 後的 padding，只跨過規定的對齊空間，因此保留前導空 argv，而不是跳過所有 NUL。對齊區若非全零會回報 invalid_data。採集前後核對啟動身分、UID 與 executable，失敗丟棄命令；程序仍可自行修改 argv，觀測不是原子快照，也不是可信啟動紀錄。

介面依據：[Apple XNU sysctl 實作](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/kern/kern_sysctl.c)、[exec 參數對齊](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/kern/kern_exec.c)。

## 共通值

除各節明示可省略的欄位（例如 tag_report／extension_details）外，下列物件欄位均會輸出；可缺值的欄位使用 `null`。空陣列使用 `[]`，不是 null。

| 型別／後綴 | 規則 |
| --- | --- |
| integer | 非負整數，除非該欄另有說明；上限見 Schema。計數／bytes／時間可到 uint64 範圍，PID／UID 為 uint32 |
| number | 有限 JSON 數字，不輸出 NaN 或 Infinity；百分比可帶小數 |
| `_bytes` | 原始 bytes，不含 KiB／MiB 字串 |
| `_seconds` | 秒；uptime 包含閒置時間 |
| `_ms` | 毫秒；`captured_unix_ms` 為 Unix epoch 毫秒，其他為經過時間 |
| `_unix_us` | Unix epoch 微秒 |
| `_percent` | 百分比數值，1.0 表示 1%；CPU 尺度見統計規則 |
| 路徑 | UTF-8 字串，不是 URL；JSON escaping 保留空白、反引號、換行等字面值。專案探索通常 canonicalize，程序 cwd/exe 保留採集值；不要假設所有路徑都有同一正規化規則 |

JSON 整數範圍可能超出 JavaScript `Number.MAX_SAFE_INTEGER`。需要完整精度的工具應使用支援大整數的 JSON parser，不能先經過 double 再轉 BigInt。`null` 表示未知或不適用，零是已知零；是否適用須結合資源型別／state 判讀。

## Snapshot

| 欄位 | 型別與意義 |
| --- | --- |
| `schema` | 固定 `reswho/12` |
| `captured_unix_ms` | 本輪收集完成附近的 Unix 時間，不是所有欄位共同的原子採樣時刻 |
| `sample_ms` | 本機兩次掃描取樣階段的經過時間；各程序觀察時間及 Docker 取樣不同 |
| `collection_ms` | 本輪採集與分類總耗時；不含輸出序列化時間 |
| `system` | 整機指標；指定 `--from` 時仍是整機 |
| `scope` | 選取範圍 ref，等於 `root.reference` |
| `warnings` | Diagnostic 陣列；含 `collection_note` 靜態限制與本輪整機／採集問題 |
| `extensions` | 本次 startup／能力執行的結構化 reports，見 reswho/10 段落 |
| `root` | 所選 Node，完整快照使用 `group:all` |

`system` 的 `cpu_percent` 為整機 0–100%，`logical_cpus` 為邏輯核心數。`memory_total_bytes`、`memory_used_bytes`、`swap_used_bytes` 為整機原始數值。五個欄位均可為 null。

## Node 與樹

| 欄位 | 型別與意義 |
| --- | --- |
| `reference` | 不透明查詢 ref；程序 `pid:N`，其他 `group:ID` |
| `name` | 展示名稱，不唯一，不用於合併／識別 |
| `kind` | `group`、`process`、`container` 或 `resource` |
| `grouping` | 程序為 null；其他為 category/runtime/container/app/repo/checkout/project/bin/system/unknown/resource；resource leaf 亦為 null |
| `evidence` | 展示用文字陣列；不得依前綴或英文內容解析路徑 |
| `attribution` | Attribution 或 null；分類的可機讀來源與路徑 |
| `diagnostics` | 分類／樹建構問題；指定 `--from` 時也包含沿路祖先的分類及 runtime 診斷 |
| `process` | Process 或 null |
| `container` | Container 或 null |
| `runtime` | Runtime 或 null |
| `resource` | typed resource 或 null；指標不納入 host／container totals |
| `totals` | 本節點與所有後代的主機程序彙總 |
| `container_totals` | 容器彙總或 null；只出現在容器 category/runtime/project 群組 |
| `children` | 子節點陣列；群組之間是歸屬關係，程序之間是經身分檢查的父子關係 |
| `omitted_children` | 因深度收合而省略的直接子節點數，不是全部後代數 |

| kind / grouping | 非 null payload | 限制 |
| --- | --- | --- |
| process / null | `process` | container/runtime/resource/container_totals 為 null；子項只可能是 process |
| resource / null | `resource` | process/container/runtime/container_totals 為 null；無 children，omitted_children 為0 |
| container / container | `container` | process/runtime/container_totals 為 null；無 children，omitted_children 為0 |
| group / runtime | `runtime`、`container_totals` | process/container 為 null |
| group / 其他 | 無單一資源 payload | process/container/runtime 為 null；容器 category/project 可有 container_totals |

容器樹上的 `totals` 是零程序的主機總計，不表示容器資源為零，應讀 `container_totals`／`container`。`--depth` 計算原始樹的邊，選取根為0；裁切不改 totals、attribution 或 diagnostics。`children=[]` 搭配 `omitted_children>0` 是收合；後者為0才表示本快照沒有可見子項，仍不保證採集完整。

JSON 保留 `group:all` 根與 System services 包裝，不套用 TUI／Markdown 的顯示精簡。完整快照中每個 ref 唯一；一般程序只在樹中出現一次。PPID 可指向範圍之外或不同分類的程序，不會為補齊父鏈而複製程序。

## Attribution

這些欄位在分類來源處寫入，不從 evidence 文字反向解析。

| 欄位 | 型別與意義 |
| --- | --- |
| `source` | 下表的來源 enum |
| `path` | 主要歸屬路徑或 null；意義由 source 決定 |
| `checkout_path` | 所屬 checkout 根或 null；不同於內層 manifest 專案根 |
| `manifest` | 探索命中的檔名，例如 `go.mod`；沒有則 null。只確認存在，不宣稱內容有效 |
| `bundle_id` | 讀得的 CFBundleIdentifier 或 null；不代表簽章驗證 |
| `inferred` | 是否透過祖先或 bind 線索推論；不是置信度或可安全停止判定 |
| `via_pid` | 祖先推論的來源 PID 或 null；非 null 時 inferred 必為 true |

| source | path 的意義 |
| --- | --- |
| `app_bundle` | 從執行檔結構找到的 `.app`／`.app.bundle` 路徑 |
| `bundle_id` | 多個 bundle 路徑依宣告 ID 合併的 app 父群組；path=null，bundle_id 非 null |
| `git_root` | 從 cwd 探索到的 checkout 根；checkout_path 同值 |
| `manifest` | 最近專案描述檔所在目錄；checkout_path 可為外層 Git 根 |
| `git_common_dir` | 多 checkout 共用的 Git metadata 目錄，通常是 `.git`；不是工作目錄 |
| `executable` | 完整執行檔路徑，只表示同一 binary，不宣稱同一專案／工作 |
| `compose` | Compose 宣告的專案路徑，可能缺失；屬宣告資料，未證明本機存在 |
| `bind_workdir` / `bind_project` | 經現有條件檢查的本機 bind 推論專案根；inferred=true |

Category／未知歸屬可為 null。一般程序摘要的分類 evidence 為 `[]`、attribution 為 `null`；指定 PID 詳情保留當輪歸屬及 `tag_report` 的多維提案與裁決。群組 attribution 描述有效分組依據，不代表所有成員都由直接線索分類。`source_priority` 決定各維度的有效提案與排除；樹再從有效 tags 依 app → project → bin 選主要分組。App／project 可同時有效，此時仍以 app 分組。Container 的專案名稱／service／mounts 另在其 payload，避免把展示名稱當路徑。

## Process 與 Network

| Process 欄位 | 意義 |
| --- | --- |
| `pid` | 程序編號 |
| `ppid`、`uid` | 父程序編號、使用者編號；未知為 null |
| `started_unix_us` | 經前後觀測確認的程序啟動時間；未知為 null |
| `uptime_seconds` | 自啟動至觀測的秒數或 null |
| `name` | 程序展示名稱；不可讀時可能是 fallback 名稱 |
| `executable`、`cwd` | 完整執行檔／工作目錄或 null |
| `cpu_percent` | 單一本機邏輯核心為100%，可超過100%；沒有有效取樣對為 null |
| `resident_bytes` | 該程序 RSS 或 null；不是它與子程序的總和 |
| `network` | Network 或 null；null 表示沒有取得網路觀察 |
| `unavailable` | Diagnostic 陣列 |

Network：`complete` 表示本次已列出 descriptor 的解析是否完整；`sockets` 為觀測到的 TCP／UDP sockets；`unavailable` 為 Diagnostic 陣列。`complete=true, sockets=[]` 表示這輪沒有看到支援的 TCP／UDP socket，不表示沒有任何網路活動。觀測非原子，沒有 Unix socket／流量／閒置歷史；同程序重複 socket descriptor 已去重。

Socket：`fd` 為非負 descriptor；`protocol` 為 tcp/udp；`local` 為 `IP:port` 或 `[IPv6]:port`；`remote` 為同格式或 null；`tcp_state` 為來源狀態文字或 null。端點目前是標準 socket address 字串，應交給語言的 socket-address parser，不用最後一個冒號以前皆為 IPv4 之類假設。UDP 綁定不代表伺服器，已連線也不代表有流量。

## Runtime、Container 與容器子結構

Runtime：`name`（目前 docker / podman / apple）、`context`、`endpoint_ref`、`daemon_id`（後三者可為 null）、`status`（available/unavailable）、`inventory_complete`、`collection_ms`、`unavailable`。available 只表示已成功列出 inventory，不保證 stats/inspect 完整。endpoint_ref 是 endpoint 的不透明識別，不暴露 endpoint 原文。

| Container 欄位 | 意義 |
| --- | --- |
| `runtime`、`endpoint_ref`、`id` | runtime 名稱、endpoint 識別、原生容器 ID（Apple 可以是名稱，非固定長度 hex） |
| `name`、`state` | 名稱、來源生命週期狀態（可 null／未識別值） |
| `started_at` | runtime 原始啟動時間字串或 null；Docker 通常為 RFC3339，但不保證來源字串有效，須檢查後才使用 |
| `uptime_seconds` | 成功解析且適用的本次 running/paused incarnation 時長，否則 null |
| `cpu_percent` | daemon 單一邏輯核心為100%；與主機取樣不同 |
| `memory_bytes` | runtime 提供的用量；Linux Docker 扣除適用的 cgroup inactive file，CLI 來源另經文字數值捨入；Apple 為原始 memoryUsageBytes |
| `memory_basis` | `docker_api_linux_cache_adjusted` 為直接 API 的整數 bytes；`docker_cli_linux_cache_adjusted_rounded` 為 CLI 捨入值；`apple_xpc_memory_usage_bytes` 為 Apple XPC 原始 bytes；讀取者須保留未知來源值 |
| `ports` | Port 陣列或 null；null 表示未取得／無法解讀，[] 表示沒有發布端點 |
| `mounts` | Mount 陣列；取得的掛載設定，缺失原因看 unavailable，不可由 [] 單獨證明完整 |
| `project` | Project 或 null |
| `candidates` | 有歧義、未確立歸屬的候選路徑陣列 |
| `service` | Compose service 名稱或 null |
| `unavailable` | Diagnostic 陣列 |

Port：`protocol` 來源字串、`container_port`（uint16）、`host_ip`（string/null）、`host_port`（uint16/null）。這是發布設定，不是觀測到的 listener／流量。

本機 Unix endpoint 的 Linux Docker stats 使用同輪兩次 one-shot，間隔沿用 `--set sampling.interval_ms=…`（預設250ms）；`cpu_percent` 是該輪樣本間的平均，沒有跨輪平滑。SSH／TCP 等端點或不支援的 API 版本沿用 CLI 的取樣方式。`sample_ms` 仍只描述主機取樣階段，不代表容器的實際取樣窗。這些來源差異不改變 JSON 欄位結構；版本目前為 `reswho/12`，指標語意保留。

Podman 使用同輪兩次 raw CLI counters，等待有效 `sampling.interval_ms`；CPU 以 CPUNano／SystemNano 差計算，memory basis 為 `podman_cgroup_usage_bytes`。SystemNano 不遞增或重驗失敗時為 unknown；macOS machine／remote bind 不推論本機路徑。實測限4.9.4 nested Linux，詳見 [容器採集](containers.md)。

Apple Container 1.6.1 以同輪兩次 XPC counters 與回應的 monotonic 時間差計算 CPU，等待沿用 `--set sampling.interval_ms=…`，無跨輪資料。未知版本只提供 inventory 並回報 unsupported；不以慢速 CLI stats 偷換取樣方式。`apple` endpoint 依本機使用者服務識別，daemon_id 為 null。

Mount：`kind` 來源類型（Apple virtiofs 映射為 bind）、`source` 與 `destination` 路徑。遠端 daemon 的路徑不是本機路徑；不自動在本機解析。

Project：`name`、`path`（string/null）、`source`（compose/bind_workdir/bind_project）。僅靠同名不合併不同 endpoint 或專案路徑。

## 統計、缺值與完整度

`totals` 欄位：`processes`、`known_cpu_percent`、`known_resident_bytes`、`cpu_unknown`、`memory_unknown`、`oldest_uptime_seconds`、`uptime_unknown`。

- totals 是本節點與所有後代的彙總，收合不影響數值。父 totals 已包含子 totals，不可再次相加。
- unknown 計數各自不超過 processes。已知部分為零且 unknown>0，不代表所有程序用量為零。
- oldest 是最長已知 uptime；全未知或零成員為 null，部分未知時只是下界。
- process CPU 與 totals CPU 以單核心100%計，system CPU 才是整機0–100%。
- RSS 共享頁可能重疊；程序 RSS 總和不能當成整機 memory_used。
- `group:all` 的 totals 排除 Containers 子樹，不能再把容器用量與主機程序相加。

`container_totals` 欄位：`containers`、`active_containers`、`inactive_containers`、`activity_unknown`、`inventory_complete`、`known_cpu_percent`、`known_memory_bytes`、`cpu_unknown`、`memory_unknown`、`oldest_uptime_seconds`、`uptime_unknown`。

- active=running/paused/restarting；inactive=created/exited/dead/stopped；其他／缺失 state 為 activity_unknown。前三種計數之和等於觀測 containers。
- inactive 的 cpu/memory/uptime 為 null、不計入 unknown；未知活動性不能當 inactive。
- cpu/memory/uptime_unknown 不超過 `containers - inactive_containers`。全 inactive 的已知總和為0、oldest=null，語意是不適用，不是缺測。
- inventory_complete=false 表示可能還有未列入的容器；containers 是已觀測數，未知計數只涵蓋已觀測成員。零容器且 inventory 不完整不代表沒有容器。

Schema 驗證型別、必填欄位、值域與節點組合；跨欄位／跨節點關係（例如 ref 唯一、scope=root.reference、總量加總、unknown 上限）由契約測試與程式邏輯保證，不能只靠通過 Schema 宣稱全部語意正確。

## Diagnostic

`warnings`、各 payload 的 `unavailable`、Node 的 `diagnostics` 均使用相同物件：

```json
{
  "code": "permission_denied",
  "operation": "identity.before",
  "message": "identity.before: Operation not permitted (os error 1)",
  "os_error": 1
}
```

`code` 是穩定原因分類；`operation` 是來源操作識別；`message` 只供展示；`os_error` 為實際取得的 OS 整數錯誤碼或 null，不從文字擷取。沒有足夠資訊時使用一般原因，不把缺測猜成程序已退出或權限不足。

| code | 意義 |
| --- | --- |
| permission_denied / not_found / io_error | 依原始 I/O error kind 分類；not_found 不等於已確認程序退出 |
| identity_changed | 前後程序啟動時間不同，已丟棄該次混合觀察 |
| identity_unverified | 容器 state/start 改變或重驗失敗，原因尚不能細分 |
| sample_unavailable | 沒有可用的同一程序身分 CPU 取樣對 |
| unavailable | 該欄位／資料缺測，未取得更精確原因 |
| collection_failed | 整個採集來源失敗，例如 Docker 初始化／inventory 錯誤 |
| command_failed | inspect/stats 指令或其完整性檢查失敗；不猜 CLI 文字背後的 OS 原因 |
| invalid_data | 來源資料無法按預期解讀或有重複／非預期記錄 |
| limit_exceeded | 已知 inventory 上限造成裁切 |
| unsupported | 該來源不支援此類指標 |
| parent_chain_incomplete | 程序父鏈循環或達深度界線，改為獨立根 |
| collection_note | 靜態採集限制，屬說明而非當次錯誤 |

目前 operation 包括 collection、containers、host.cpu、host.logical_cpus、host.memory_total、host.memory_used、host.swap、identity、identity.before、identity.after、task_counters、cpu、network、network.socket、executable、cwd、repository_metadata、parent_chain、docker、docker.inventory、docker.inspect、docker.stats、apple、apple.inventory、apple.stats。此欄是可擴充字串；未知 operation 應顯示 message 或套用 code 的一般處理。

## 身分與跨快照比較

- ref 是查詢 selector，不是永遠存在的資料庫主鍵。group ref 為分類來源產生的不透明值，不要解析 hash 或自行重建。
- 同一主機、同一分類來源與契約下，group ref 可重複查詢；重分類、路徑改變、runtime endpoint/daemon 改變或資源退出，可能使 ref 改變或消失。單一／多 checkout 或 bundle 的父群組也可能出現／消失。
- `pid:N` 只指定目前 PID；PID 會重用。跨快照匹配須使用「使用者管理的主機識別 + pid + started_unix_us」，start=null 時不能確認是同一程序。不要只用名稱或 PID 判定持續存活。
- 容器在所屬 runtime／endpoint／daemon 範圍內以完整 ID 識別。容器 restart 不一定改 ID；比較 started_at 才能區分 incarnation，缺失或無效時保留未知。
- 程序快照沒有主機 UUID／boot ID，也沒有觀測 session ID；不能只靠此輸出跨主機或時鐘重設建立永久身分。

## 從 reswho/7 遷移

- `warnings` 與所有 `unavailable` 由 string[] 改為 Diagnostic[]。顯示舊訊息改讀 `.message`，判斷原因改讀 `.code`／`.operation`。
- Node 新增 `attribution`、`diagnostics`。路徑應從 attribution／process／container 的結構欄位取得；不再解析 evidence。
- 原有 ref、kind/grouping、數值單位、主機／容器統計範圍及 --depth 語意保留。`evidence` 仍是展示文字。

## 範例與處理

四份皆為虛構資料，ref 僅示範格式，不對應本機資源：

- [host.json](examples/host.json)：主機 → app → 程序父子樹。
- [collapsed.json](examples/collapsed.json)：子項收合，完整 totals 保留。
- [containers.json](examples/containers.json)：runtime／Compose、active／inactive／未知活動性。
- [unavailable.json](examples/unavailable.json)：權限診斷、null 與 unknown 計數。

取得可見程序的 raw metrics：

```sh
reswho --json --depth all | jq '
  if .schema != "reswho/12" then error("unsupported reswho schema") else . end
  | .root | recurse(.children[])
  | select(.kind == "process")
  | {ref: .reference, pid: .process.pid, start: .process.started_unix_us,
     cpu: .process.cpu_percent, rss: .process.resident_bytes}
'
```

取得群組已確立的歸屬路徑（程序 tags 須另以 `--from pid:N` 要求詳情）：

```sh
reswho --json --depth all | jq '
  .root | recurse(.children[])
  | select(.attribution.path != null)
  | {ref: .reference, source: .attribution.source,
     path: .attribution.path, inferred: .attribution.inferred}
'
```

`cargo test` 會以離線 Schema 驗證範例、實際建構／序列化的節點與收合／選取結果，並驗證錯誤型別、缺欄位、不合法組合會被拒絕。原生 live 測試另驗證本機採集輸出符合 Schema。

## 從 reswho/8 遷移至 reswho/9

Attribution source 的封閉 enum 新增 `extension`；其 `extension` 物件保存 provider、version、key、label、basis，path 仍在 attribution.path。該版保留當時 app／project 證據優先序、PID metrics 與 totals；目前分類裁決見 reswho/12 段落。Process 可新增 `extension_details` 陣列，各組包含 provider、version 與 parsed typed fields（key、label、value、sensitive）；只在 selected-PID details 查詢時提供，敏感值已由核心改成 `[filtered]`。原有 command DTO 與 native redaction 保留。

[reswho-8.schema.json](../schemas/reswho-8.schema.json) 原樣保留供舊 snapshots 使用；它不接受新版本或新 attribution source。目前 examples 與 `--json-schema` 使用12；本段保存8→9的遷移背景。消費端應先檢查 schema；不能把新資料宣稱為8或假設所有來源都屬既有 enum。Schema 驗證 current examples、custom attribution／details 及錯誤來源的完整 snapshot。

`reswho extensions --json` 是獨立的 metadata 陣列，不套用 snapshot Schema，也不執行 provider 或探測來源服務。外部 manifest、host CLI engines、JSON wire、設定範圍及 runtime DTO 的使用方式見 [extension-protocol.md](extension-protocol.md)。

## reswho/10：目前 extension report

Extension report 於 reswho/10 加入；目前為 reswho/12，Schema8／9／10／11 保留供舊資料使用。新增頂層 `extensions` 陣列，最多2048筆，每筆有 provider、nullable version、phase（read／validate／register／collect／classify／details）、result（success／failed／skipped／never_invoked）、nullable safe reason code 與 nullable duration_ms。實際嘗試使用非負數毫秒（可含小數），skipped／never_invoked 固定 null。Result 包含核心驗證，非單純 child exit。collection_ms 仍為整輪 wall time；重疊／平行 phase 不可加總。Report 只含 startup 與本輪資料，不保留跨輪執行歷史；不包含 raw stderr／panic／設定值或未要求的 command 資料。


## reswho/11：補充角色與一般資源

`process.roles` 可省略或為最多64項的陣列（跨來源）；超限拒絕該來源整份 contribution，不裁切。role 保存 provider／version／key／label／basis／absolute path。它與 node.attribution 的主要歸屬分開；不重複 PID、不改父子關係或 totals。相同 provider/key 每 PID 唯一，不同 provider 並存，依登記順序呈現。

Node 新增 nullable `resource`，`kind: "resource"` 的 leaf 使用非空 resource，process／container／runtime／container_totals 為 null，grouping 為 null。它位於 `group:resources` 的 provider 下；category／provider group 使用 `grouping: "resource"` 保存 metric scope，與 children 是否省略無關。Resource 包含 provider／version／id／kind／label／nullable path／metrics；metrics 是 key／label／unit／value。Number 保留精確 JSON，null 是未知，缺少整個 metric 是未觀測。Unit 是有界 identifier 字串，可擴充，不自動換算或聚合。Resource node 的 totals 固定為零／unknown uptime，不代表量到零個硬體程序；其語意是不屬於 PID 計量。Host 與 container totals 維持既有規則，完全不加 resource 指標。CPU／RAM／uptime sorting 不對 generic metrics 猜測映射。

外部 wire protocol 的版本與 DTO 不受 snapshot schema 版本綁定，見 [extension protocol](extension-protocol.md)。


## reswho/12：共同摘要／PID detail 投影

內部分類可保存完整 tags 與裁決，但公開 Snapshot 在 renderer 前以同一投影控制 scope。一般樹只有分類／路徑／資源摘要，所有 process 的 tag_report 欄位省略，也沒有舊 process.roles 或 process evidence／attribution 的旁路。TUI、Markdown、JSON 相同。

明確 `--from pid:N` 或 TUI 開啟指定程序 details，才取得該 PID 的 `tag_report: {effective, proposals, decisions, diagnostics}`；effective 保存 source＋tag，proposals 保存 source／version／原提案，decisions 保存 source／proposal_id／status／by_source／reason。Status 為 effective／excluded／superseded／withdrawn／recorded／rejected。同來源矛盾保留 rejected 原提案及安全診斷；未通過界限驗證的 payload 不回傳。資料僅當輪，無持久歷史。

未要求詳情是 field absent；已要求且無提案則有四個 typed 空陣列。Identity 不可用、來源失敗／停用／未採集時有 diagnostics，不假稱完整空集合。子程序不論 depth 都用摘要，Schema 也拒絕 child tag_report。Tag 完整身分與 root／宣告／優先序見 extension-protocol.md。

Schema11 的 process.roles 是歷史契約；Schema12 將它移入 tag_report，有效標籤不造成 PID／CPU／RAM 重複計數。Generic resource 及 host/container 計量範圍不變。

Ownership group ref 統一由 dimension／id／canonical root 決定，不包含勝出來源或顯示名稱；內建與外部同身分可合併，不同 ID／root 保持分離。App／Git metadata 僅裝飾及組織階層，不另選主要 ownership 身分。此修正會改變早期原型的 ownership group refs；PID refs 不變，請從目前 snapshot 重新取得 group ref。
