# Podman／Apple Container 實機探測

2026-11-02。以下先記錄 adapter 實作前的協定探測；Apple 已完成接入，驗收見文末，Podman 尚未接入。

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
