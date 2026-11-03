# Podman／Apple Container 實機探測

2026-11-02。以下先記錄 adapter 實作前的協定探測；Apple 與 Podman 已完成接入；早期探測與接入驗收分列於後。

## Apple Container

本機 macOS 26.5.2、arm64，Apple Container 1.6.1（aa8d76b）。安裝後服務未啟動；本輪啟動服務並安裝預設 kernel，建立一個 1 CPU／256 MiB 的 BusyBox sleep 容器，唯讀掛載自有 Cargo 專案目錄。完成後刪除測試容器。

| 查詢 | 單次實測 |
| --- | ---: |
| inspect | 31 ms |
| list --all --format json | 17 ms |
| stats --format json --no-stream，第一次 | 2049 ms |
| stats --format json --no-stream，第二次 | 2098 ms |

以上是單一存活容器的探測數據，不是效能基準或多 runtime 採集時間。

- list 與 inspect 都提供 configuration、status；原生 id 可以是名稱，不是 Docker 的 64 位 hex。1.6.1 的狀態位於 status.state，啟動時間為 status.startedDate。
- configuration 包含 labels、initProcess.workingDirectory、mounts、publishedPorts，可作歸屬依據。也包含環境變數與命令，因此 adapter 必須只映射需要的欄位，不直接輸出原始 JSON。
- 本輪 bind mount 以 `type.virtiofs` 表示；不能把所有 filesystem attachment 當成 bind。
- stats JSON 提供 memoryUsageBytes 與累積 cpuUsageUsec，沒有 CPU 百分比或取樣時間。
- 1.6.1 CLI 即使指定 JSON／no-stream，內部仍取兩次 stats、等待 2 秒，最後只輸出第二份 counters。兩次獨立 JSON 查詢本輪共約 4.15 秒；不能把累積 CPU counter 直接當成百分比。
- table 輸出利用內部兩次取樣顯示 CPU%，但 memory 經格式化與捨入。實作前須明確選擇資料口徑、失敗語意與延遲，不能宣稱 JSON 已提供現成 CPU%。

來源：[1.6.1 CLI stats](https://github.com/apple/container/blob/1.6.1/Sources/ContainerCommands/Container/ContainerStats.swift)、[stats 資料型別](https://github.com/apple/container/blob/1.6.1/Sources/ContainerResource/Container/ContainerStats.swift)、[ManagedContainer](https://github.com/apple/container/blob/1.6.1/Sources/ContainerResource/Container/ManagedContainer.swift)。

### 直接讀取 XPC counters

2026-11-03，同一台主機與 Apple Container 1.6.1。以暫時的 C 探測程式使用系統 libxpc，呼叫 `com.apple.container.apiserver` 的 `containerStats`，讀取 `statistics` JSON；每次啟動讀兩次，中間等待 250 ms，不保留跨輪資料。

自有 BusyBox 容器限制 1 CPU／256 MiB，以最長 60 秒的 busy loop 產生負載。一次 CLI 對照與一次 XPC 配對取樣結果：

| 查詢／量測範圍 | 耗時 |
| --- | ---: |
| CLI stats JSON／no-stream，含程式啟動 | 2159.75 ms |
| XPC 兩次取樣與等待，含探測程式啟動 | 659.98 ms |
| XPC 第一次／第二次請求本身 | 7.80／9.34 ms |
| 兩份回應的 monotonic 時間差 | 261.28 ms |

本輪端到端約快 3.3 倍；不把請求時間或 250 ms 等待當成整輪耗時，其餘啟動等開銷未單獨拆解。250 ms 取樣與 CLI 的 2 秒平均窗口不同，CPU 更容易波動。

以累積 CPU 微秒差／實際 monotonic 秒差換算，單核心口徑為 99.66%，第二份 memory 為 4,280,320 bytes。探測前後確認相同容器的 creationDate／startedDate 未變且仍為 running；檢查兩份 id、counter 遞增、非零 memory，以及不存在的 id 回報錯誤而非零用量。外層 subprocess timeout 為 CLI 8 秒、XPC 5 秒。

這證實固定 2 秒等待可避開，尚不是 reswho 的效能改善：目前沒有接入 adapter，也未測多容器、多 runtime、其他版本或持續刷新。正式採用仍需處理有界並行、取消／逾時、容器重啟與協定不相容；不能把 Apple Container 專案的 XPC 訊息格式視為跨版本穩定承諾。

完成後移除自有容器、新拉取的 BusyBox image 與暫存探測程式，服務恢復原本停止狀態；既有 kernel 保留。沒有變更 reswho 程式碼，因此未重跑 Cargo 測試。

協定來源：[ContainerClient.stats](https://github.com/apple/container/blob/1.6.1/Sources/Services/ContainerAPIService/Client/ContainerClient.swift)、[XPC 路由／欄位](https://github.com/apple/container/blob/1.6.1/Sources/Services/ContainerAPIService/Client/XPC%2B.swift)、[XPCMessage](https://github.com/apple/container/blob/1.6.1/Sources/ContainerXPC/XPCMessage.swift)。

## Podman in Docker

Docker Engine 29.9.2／Linux arm64。官方 `quay.io/podman/stable:latest` 的本輪 digest 為 `sha256:b6b2f33e559cd36ab31a4b54770ff4e09a55df555eef9e731159003725eda04e`，內含 Podman 5.8.9、crun 1.28。

外層使用自有標籤的 privileged 容器，限制 1 CPU／768 MiB，不掛載 Docker socket 或主機目錄。內層使用 128 MiB 的 BusyBox sleep 容器。每次嘗試均核對外層 ID／label，再刪除外層與匿名 volumes。

環境診斷結果：

- 映像預設 `cgroups="disabled"`，stats 曾回報 `this container does not have a cgroup`。
- 改成 `--cgroups=enabled` 後，預設儲存方式仍使內層程序立即退出，log 為 `exec container process /bin/sleep: Invalid argument`。此時 stats 命令即使成功返回零值，也不能視為存活容器的用量驗證。
- 最終以 `podman --storage-driver=vfs --cgroup-manager=cgroupfs run --cgroups=enabled --network=none ...` 成功維持內層程序存活；inspect 確認 Running=true，再驗證 stats。

成功那次的 list／inspect／stats 分別為 41／42／39 ms，包含 docker exec 與 CLI 啟動。stats 返回 1 個 PID、非零 memory 與累積 CPU 時間。這是單輪有效資料驗證，不代表持續採樣 CPU 精度或負載下的效能。

- 預設 stats JSON 的 id 是縮短識別，memory 是十進位格式化字串（本輪 `503.8kB / 134.2MB`），不能沿用 Docker 的完整 ID／二進位單位假設。
- 此環境的 system connection list 是空陣列，只驗證 Linux 本地 Podman。macOS podman machine、SSH connection、rootless、VM mount 路徑對應仍未驗證。
- 不同 runtime 應保留自己的狀態與失敗原因；多 runtime 並行與整體採集時間尚未測試。

來源：[Red Hat：Podman inside a container](https://www.redhat.com/en/blog/podman-inside-container)、[Podman 疑難排解](https://github.com/podman-container-tools/podman/blob/main/troubleshooting.md)。

## 後續驗收

fixtures 用於缺欄位、格式改變、逾時、部分失敗與身分變更；explicit live 測試另外驗證真實 CLI、存活容器與採集耗時。一般 cargo test 不啟動 runtime 或容器。

本輪只驗證測試環境與來源資料，未宣稱 adapter、TUI／MD／JSON 整合或多 runtime 效能已完成。

## Apple adapter 接入驗收

2026-11-03，Apple Container 1.6.1、macOS 26.5.2 arm64。正式 snapshot 路徑的 debug build，預設250ms，無 cache；兩個自有1 CPU／256 MiB運行中容器（busy loop、sleep）及一個未啟動容器，另唯讀查詢既有 Docker。CPU 單核心口徑，idle 容器實測0%，busy約100.26%；stopped 沒有偽造用量。

完整通過那輪：Apple inventory＋XPC雙取樣＋重驗357ms，Docker588ms，整機 snapshot892ms，含執行檔啟動／JSON輸出902ms。另一次 stats/歸類通過的前輪為Apple438ms、Docker630ms、snapshot936ms，但測試腳本誤用ref欄位而未完成MD檢查；修正為reference後重跑完整通過。這是小樣本功能驗收，不是長時間或大量容器效能基準。

通過：真實stats、bind專案歸類、published port、運行／停止計數、JSON與MD查詢、合成環境秘密不進入輸出；服務原先停止時查詢後仍保持停止。原生C合成reply測試以AddressSanitizer檢查成功／錯誤／超大payload、pending與取消後晚到callback的記憶體生命週期，無需外部服務。

所有自有測試容器、新拉取BusyBox image已刪除，Apple服務還原停止；暫存探測檔案於驗收後移除。既有Docker工作未啟停，target保留為專案開發cache。其他Apple版本、多於4個Apple存活容器及長時間刷新仍未實機驗證；bounded concurrency、deadline、partial failure和重啟拒絕以fixtures覆蓋。

最終檢查：113項一般Rust測試通過，fmt／all-target Clippy通過；native合成reply的AddressSanitizer檢查通過。新增TUI TestBackend驗證Apple runtime與stopped篩選，未重跑未修改的主機PID/socket live或PTY流程。暫存目錄76KiB已移除；target約4.8GiB保留供下一輪開發，下一階段再檢查大小。

## Podman external-first：公開協定實驗

2026-11-03，base `7fc2f77`，macOS arm64 native Rust collector。先以 provider ID `podman`、protocol1、containers scope 與明確 `sampling.interval_ms` use，透過一般 manifest install 及 production dispatcher 採集。collector 不讀完整 snapshot；外部 wrapper 明確映射公開 wire DTO。後續內建移轉沿用同一 collector，builtin ID 保護拒絕殘留同名外掛，不引入 alias／replaces 或特殊來源去重。

原先記錄的5.8.9 pinned digest 本輪 pull 回覆 `not found`；只嘗試一次，未啟動容器。改用原已存在的 `quay.io/podman/stable:v4.9.4`，image ID／digest `sha256:8e3d1eaf1a55c44746229c5cb4c96bcf5cd6e657b6588204259921aeb587c64d`，不移除既有 image。

外層自有唯一名稱／label，privileged、1CPU／768MiB／512 PIDs、最長540秒，沒有 host mount／socket／published port。內層採既有已證實的 vfs／cgroupfs／cgroups-enabled，兩個128MiB運行中 BusyBox sleep 與一個未啟動容器；bind 來源僅為外層內的新建路徑，Compose labels、環境秘密均為合成資料。native macOS bridge 僅將 collector 唯讀 argv 經 docker exec 傳到確切自有 ID，沒有虛構 connection／metadata，也不是 production Docker dependency。

真實外部路徑通過：三個完整原生 ID／預期名稱與狀態、running CPU 與非零 raw memory、inactive nulls、published18080、Compose project／service、秘密不進輸出、單一成功 collect report，以及 MD profile。Podman VM／遠端 bind path 不參與本機檔案歸屬。未將容器數據加進主機程序總量。

驗證中先修復 harness 的 nullable runtime traversal；接著發現 Podman4.9.4 的 inspect normalization 只改寫單獨 `{{.Id}}`，不改寫 Docker template 的 `{{json .Id}}`。Podman 改用 `.ID`，仍共享 inspect parser／ports／Compose mapping；直接窄 template preflight 與離線回歸通過。最後原 `sample_ms == 400` assertion 不符合既有契約：該欄位是主機取樣經過時間；之前已通過的真實資料斷言保留，未為此重跑外部容器流程。

設定傳遞另以完全離線 production native 外掛＋自有原生 fake CLI 驗證：config／resolved setting400ms，三個合成容器恰好兩次 stats，monotonic 呼叫間距455.578ms；snapshot 主機 elapsed sample433ms，capture1712.69ms。這證明公開 request→設定→collector wait，不是 Podman CPU 精度或效能基準。PATH 僅含自有 fake CLI，未連 Docker／Apple；失敗的 Docker runtime 不影響 Podman。

每輪 finally 核對 exact outer ID／label 後刪除自有 container／匿名 volumes／內層 storage，確認 inspect 不存在且 owner label 查詢為空；所有 temp root、native binaries、pipes 與 child 已回收。既有4.9.4 image與其他工作保留。失敗輪次未當作完整驗收。

來源：Podman [stats4.9.4](https://github.com/containers/podman/blob/v4.9.4/cmd/podman/containers/stats.go)、[inspect normalization4.9.4](https://github.com/containers/podman/blob/v4.9.4/cmd/podman/inspect/inspect.go)、[raw counters5.8.0](https://github.com/containers/podman/blob/v5.8.0/libpod/stats_linux.go)、[no-stream5.8.0](https://github.com/containers/podman/blob/v5.8.0/pkg/domain/infra/abi/containers.go)、[connection precedence5.8.0](https://github.com/containers/podman/blob/v5.8.0/cmd/podman/root.go)。


### Podman builtin 接入驗證

外部實驗的可重現 source checkpoint 為本地 commit `3a72356`。完成公開 manifest／protocol 路徑後，移除臨時 wrapper／manifest，直接將同一 collector 註冊為 Builtin::Podman；一般 native example 不受影響，沒有兩份 parser／transport。

最後一次自有4.9.4 fixture 的 production builtin JSON／MD profile 通過：三個預期完整原生 ID／名稱／狀態、兩個 running 的0.0% CPU與122880／118784 bytes、created 的 nulls、raw memory basis、published port、mount／Compose／service、秘密省略、單一成功 collect report。這是 sleep 工作的功能驗證，不是 busy CPU 精度或負載基準。新 fixture 的 IDs 與前輪不同；依各輪建立時記錄的完整 IDs 比對，metadata／狀態語意與外部階段一致。

設定400ms，native bridge 記錄的兩次真實 stats 呼叫間隔481.54ms。Podman collection1182ms、整機 snapshot1751ms、JSON CLI wall2313.9ms、另一輪 MD CLI wall1523.3ms；同一 snapshot 斷言既有 Docker runtime 維持 available。主機 elapsed sample431ms。沒有同時 build 或 benchmark；這是單次小 fixture 成本觀察，不能推論大量容器或長時間效能。

最後 exact outer `fb68ae7a487a7ab723cf2ef010cada54c4d1659d865d14810cc4f856268b644b`、匿名 volumes、內層儲存與 temp root 均已移除並核對不存在；所有前輪亦已清理。既有4.9.4 image與 Docker 工作保留，未啟動或停止 Apple service。5.8.9 pinned image不可用、新版Podman、macOS machine／SSH／rootless仍未實測。

外部→內建揭露共用 registry 相容性缺口，已做通用修復：舊 persisted external 若與新增 builtin ID 衝突，仍完整驗證 metadata，但只將該外部記錄 effective blocked，reason=`external_builtin_id_conflict`。保留原 entry／order／enabled／tombstone、canonical shared definitions、使用者設定與 priority，load／startup 不因此改寫檔案。Inventory 只有一個 builtin row；其 validate report 明示阻擋的是 external，舊外部 version 與本次 builtin collect timing 分開。移除舊外部記錄後不再產生該碰撞錯誤。新 explicit install／drop-in 仍拒絕 builtin ID，損毀 state／重複 persisted ID 仍 fail closed；健康 external 與 builtin 不受影響。

最終 affected gate：fmt／all-target Clippy／diff-check 通過；Podman15例（含共用 Schema／MD 與 host/container 總量分離）、Docker mapping9例、extensions59例、CLI11例、JSON contract7例、既有 priority dialog1例的有效證據齊全。Extensions 初輪58/59通過，既有 sleeping-provider2秒期限案例因 PID file尚未出現失敗；CLI 初輪10/11通過，既有 FIFO1秒 watchdog失敗。兩個未修改的案例隨後單獨 serial執行、保留原 oracle／deadline 後通過；可能與平行啟動負載有關，未證明根因，沒有宣稱初輪平行 gate成功。修正新增 builtin 插入位置：Podman 追加到原清單末端，既有 priority canary 不修改即可通過；受影響離線清單／CLI重驗通過。未跑 full suite、PTY、未修改的 Apple／host live 或額外 stress。

Outer250ms deadline 的 nested native fixture確認 supervisor回收 provider／CLI／其子孫與內層 guard；另覆蓋 outer host death、inner timeout、cancel、missing CLI。暫存 native fixtures／pipes均已回收。兩個臨時驗收 harness及217個自有 prototype cache entries（27.4MiB）已刪除，所有已記錄 temp roots確認不存在。專案主要 target約13GiB刻意保留供後續 review／開發，由專案下一階段邊界檢視；未做全域 cache清理。Tracker與獨立review由leader處理，僅本地commit、不push。
