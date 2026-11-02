# 容器採集

已實作 Docker 與 Apple Container 唯讀 adapter。Tenqo：rw-9、rw-26。Markdown `reswho-md/18`，JSON `reswho/8`。Podman 尚未接入。

## 連線與範圍

透過 PATH 的 `docker` CLI，依 Docker 的有效設定連線：非空 `DOCKER_CONTEXT` 優先，其次 `DOCKER_HOST`，否則讀取目前 context。保留 Docker 原有的 TLS／設定目錄行為；不寫入 context，不預設 socket 路徑，不啟動 runtime。查詢使用固定 argv，沒有 shell 插值；stdin 關閉、無控制終端機，不要求輸入密碼。

唯讀命令包含 context show／inspect、info、container ls／inspect／stats。inspect template 只輸出所需欄位及 Compose project／working_dir／service labels，不讀取環境變數、命令參數或全部 labels。原始 CLI stderr 可能含連線憑證，因此只輸出 exit status／診斷存在的狀態，不轉印其內容。

一輪 CLI 與直接 API 共用 10 秒期限，每個 CLI stdout／stderr 上限 8 MiB；每個 runtime 最多顯示 256 個容器，CLI 每批最多 32 個。超時會終止並回收本輪擁有的 CLI process group，已取得的 inventory／metadata 可保留。這是容器查詢預算，並非整機採集或檔案系統查詢的硬即時保證。一般測試用 fake runner、自有短命程序與暫存 Unix socket，不連接使用者 Docker。

本機 Unix endpoint 的 Linux stats 透過 libcurl multi 直接查詢 Engine；metadata 仍使用原 CLI。每輪查 `/version` 協商 API 1.41～1.56，遵守 server minimum 與非空 `DOCKER_API_VERSION`。不支援的版本、缺少 version endpoint 或非 Unix 連線沿用 CLI；傳輸、權限或資料錯誤明示 unavailable，不再自動追加另一輪 stats。每次 GET 最多2秒，且受整輪期限限制；單一 body 上限1MiB、headers 上限64KiB，每批最多4個請求，回應逐批解析。禁止 redirect／proxy，使用 context 已解析的 socket，不假設安裝路徑。事件迴圈約每20ms檢查取消，無額外常駐執行緒或跨輪連線／資料快取。

## Apple Container

PATH 有 `container` 時加入 `apple (local)`，用 `--version` 與 `list --all --format json` 取得版本與 inventory。服務不可用時顯示 unavailable，不執行 system start。Docker 與 Apple 在各自短期 worker 中並行，共用10秒容器查詢 deadline，結束或取消時 join；`--no-containers` 跳過兩者。

已驗證 1.6.1 的 XPC stats 協定：直接向本機使用者的 apiserver 送出唯讀 containerStats，最多4個在途請求，單筆最多2秒且受整輪 deadline 限制。C bridge 僅處理 libxpc 的 block callback 與至多64KiB stats 資料，Rust 每5ms檢查完成／取消；釋放 request 時取消 connection，晚到的 callback 有獨立存活期，不引用 Rust stack。不產生 helper process、不使用跨輪 cache。

每輪取兩次累積 cpuUsageUsec，中間等待 `--sample-ms`，用實際回應的 monotonic 時差換算單核心 CPU%；memory 原樣使用 memoryUsageBytes，標示 `apple_xpc_memory_usage_bytes`，不混用 Docker 的 cache-adjusted 口徑。再次列出 inventory，比對 id、creationDate、startedDate 及 running；觀測到改變或無法重驗就丟棄用量。CLI 的日期精度有限且不是原子快照，不能排除兩次觀測間未呈現的狀態變化。

XPC 訊息格式沒有跨版本穩定承諾，因此目前僅1.6.1走此路徑，其他版本保留 inventory 與 unsupported 診斷，不追加慢速 CLI stats。Apple 原生 id 可為名稱；ref 保留 runtime／使用者 endpoint 範圍，不與 Docker 同名容器合併。stopped 為 inactive，stopping／unknown 維持活動性未知。

只從 CLI JSON 映射需要的欄位；環境變數、命令參數與其他 labels 不進入輸出。virtiofs 掛載視為本機 bind 路徑，其餘掛載保留來源類型；專案分類沿用下方規則。published port range 有界展開，無法解讀時保留 unknown。實機數據與限制見 [runtime probes](runtime-probes.md)。

## 分類規則

| 證據 | 分類方式 | 邊界 |
| --- | --- | --- |
| `com.docker.compose.project` | 直接建立 Compose 專案群組 | 屬於宣告 metadata；working_dir 保留為宣告路徑，不要求目前存在 |
| bind mount 對應 container working directory，且本機找到專案標記 | 推論專案，來源 `bind_workdir` | Docker Unix socket 或本機 Apple XPC；採最深 mount，避免父掛載覆蓋巢狀 mount |
| 所有可辨識 bind 路徑只指向一個專案 | 推論專案，來源 `bind_project` | 即使沒有 working directory 也保留這項有條件的線索 |
| 多個不同專案候選，無 working directory 可區分 | 維持容器層級 | 保存 candidates，供深入查詢 |
| remote endpoint、無標記、缺路徑、named volume | 不以這些路徑推論本機專案 | 不因同名、image 名稱或名稱前綴強制歸類 |

本機專案標記沿用程序分類的 21 種 manifest／Git 規則。Unix socket 不保證 daemon 一定共享本機檔案系統，例如可能是遠端代理；所以 bind 路徑分類始終標示 `~`，不當成直接歸屬證據。Compose 宣告優先，其他可用路徑仍留在 candidates。路徑有衝突不合併成同一主機專案，容器與主機資源維持各自統計範圍。

同名容器、不同 endpoint、不同 daemon 不共用 ref。容器 ref 依 runtime、endpoint hash、daemon ID、完整原生 ID 產生；與名稱、顯示位置、專案分類無關。專案群組按來源類型、名稱及路徑分開，不只靠 basename。

## 資料與統計

- 容器行：`name [container] group:REF · CPU  memory  uptime  state  published ports`。
- runtime／專案群組顯示 `active / containers`，JSON 的 `container_totals` 分開記錄 `active_containers`、`inactive_containers`、`activity_unknown`；原 `totals` 只統計主機程序。全機 Resources 不加上 Containers 子樹用量，避免 VM 與內部用量重複計算。
- 支援直接 API 時，先取得所有容器第一筆 one-shot，再等待一次 `--sample-ms`（預設250ms），取得第二筆。CPU 使用計數差值；缺值、計數回退、核心數變動、無效或不遞增時間戳皆不冒充0。沒有跨輪平均／平滑；實際每個容器的取樣窗可能因查詢耗時而更長。
- CPU 100% 為 daemon 的一個 logical core；memory 扣除 cgroup v1 `total_inactive_file` 或 v2 `inactive_file`，非主機程序 RSS。API 的整數 bytes 標為 `docker_api_linux_cache_adjusted`；CLI fallback 使用 `docker stats --no-stream --no-trunc` 的四捨五入值，標為 `docker_cli_linux_cache_adjusted_rounded`。`memory_basis` 保持開放字串，不更動 JSON schema 版本。
- 採集所有生命週期狀態的容器；只對 running／paused 取得 stats。created／exited／dead／stopped 的 CPU／memory／uptime 不適用，以 `-` 顯示，不計入 unknown 與 oldest；JSON 原指標維持 null，不偽造零值。running／paused／restarting 計入 active；removing、未知或缺失狀態的活動性維持未知，不能當成 inactive。啟動時間原字串仍保留。
- 完整且全為 inactive 的群組用量與 oldest 為 `-`；若仍有狀態不明的成員，用量保持 `?`／部分總和。活動性未知或 inventory 不完整時，active 計數保留 `?`／`>=`。
- stats 前後重新檢查完整 ID、狀態及有效 StartedAt。變更或不可重驗時丟棄該次用量；取樣不是原子快照，也不代表持續閒置時間。
- Published ports 是設定映射，不代表容器內確實有 listener。摘要 `tcp:9000->8080`，詳情保留 host IP；JSON 也保留未發布的 exposed ports。host network 的 ports 標示未知。
- 沒有 CLI、連線失敗、超時、metadata 缺失都不阻斷主機採集。完整空 inventory 為 0；失敗 inventory 為 `?`，超過上限為 `>=`。未知量與真實零不同。

TUI 與一次性輸出都在每輪重新查詢完整 metadata 與用量，不使用跨輪 Docker metadata 快取。畫面呈現最近完成的快照，採集期間仍顯示上一輪資料；不是連續即時串流。

## 查詢

```sh
reswho --md --from group:containers --depth 3
reswho --json --from group:containers --depth all
reswho --md --from group:REF --depth 0
reswho --md --no-containers
```

REF 替換為輸出中的容器 group ref。選取容器時顯示 started time、service、mounts、project candidates、published endpoints 及診斷；一般展開維持摘要。Compose 依據與路徑合為群組的一行 `> compose PATH`，缺路徑時保留 `> compose`；名稱已在群組列，不重複。主機 root → Containers → runtime → project → container 每個邊都計入 depth；無專案時省略 project 層。

參考：[Docker lifecycle states](https://docs.docker.com/reference/cli/docker/container/ls/#status)、[Docker contexts](https://docs.docker.com/engine/manage-resources/contexts/)、[Docker stats](https://docs.docker.com/reference/cli/docker/container/stats/)、[Compose labels](https://github.com/docker/compose/blob/main/pkg/api/labels.go)。
