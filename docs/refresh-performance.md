# 刷新成本

## 直接 stats API 實作後的完整採集

2026-11-02，rw-23，基於 `14a2de7` 加入本機 Unix endpoint 的直接 API。每輪重新協商版本、取得兩筆 one-shot，相隔預設250ms；metadata 仍每輪查詢，無跨刷新快取／平滑。最多4並行，使用 libcurl multi，不為容器增加程序或常駐執行緒；HTTP body 逐批解析，避免累積全部原始回應。

同一台 macOS 26.5.2 arm64、Docker CLI／Engine 29.9.2、一般使用者在工具沙箱外，未用 sudo；debug test build 已預先建置。`containers::measure::measure_full_snapshot` 透過正式 `snapshot::capture` 串行量測三輪，總用時3.07秒，預算40秒；沒有同時執行建置／其他量測。

| 指標 | 第1輪 | 第2輪 | 第3輪 |
| --- | ---: | ---: | ---: |
| 完整採集（主機＋Docker） | 1,173ms | 929ms | 964ms |
| 其中 Docker 採集 | 859ms | 620ms | 622ms |
| 主機取樣階段 | 263ms | 264ms | 277ms |
| 主機程序數 | 780 | 780 | 780 |
| 容器數／active | 13／4 | 13／4 | 13／4 |
| 容器 CPU／memory unknown | 0／0 | 0／0 | 0／0 |

三輪容器 inventory 完整、runtime／container 診斷皆為0；active memory basis 全為 `docker_api_linux_cache_adjusted`，證實走新路徑。主機 warnings 計數各為3；採集程式固定包含3則 `collection_note` 說明，這輪沒有額外 host summary 警告，但不代表每個程序都可完整讀取。

量測不含畫面繪製、CLI 啟動或 JSON 編碼。先前含 Docker 單次採集3,018ms、CLI-only Docker 三輪1,994～2,705ms 是不同時間／容器數的歷史觀測，不能作嚴格百分比比較；本次也未獨立量測正式 API stats 階段或 daemon／VM CPU 開銷。較多容器、遠端連線與其他 runtime 的效能仍未驗證。API 短取樣窗可能比 CLI 約1秒平均更敏感，沒有跨輪穩定化。

驗證：87項一般測試通過；最後將 HTTP 回應改為逐批解析後重跑受影響的32項容器測試，更新 help 後重跑3項 CLI 測試，皆通過。fmt、all-target Clippy、3項 PTY smoke 通過；PTY 覆蓋主機模式的載入、鍵盤／resize、q／Ctrl-C 與終端機復原。API 的 stalled socket 取消、deadline、HTTP chunked／截斷／大小上限、版本 fallback、CPU／memory 邊界及整條採集的重啟競態由自有 Unix socket fixture 驗證。初次大小測試因 macOS fixture 接受的 socket 繼承 nonblocking 而截斷，已明確改為 blocking；deadline 測試亦促使實作區分整輪期限與單次 HTTP timeout。未重跑未變動的原生 process/socket live tests。

正式 JSON 保持 `reswho/8`，`memory_basis` 原契約為開放字串，新增 API 來源；Markdown 結構不變並更新 `--help-format`。所有自建 CLI／PTY 程序、socket fixture／執行緒皆回收，未啟停外部服務、未保存容器／程序明細。新增 curl crate（不啟用預設 TLS 功能）；`target/` 約3.7GiB，保留供本專案後續開發。重現方式見 [TESTING.md](../TESTING.md)。

## Docker stats 無快取替代方案

2026-11-02，rw-22，基於 `5a3107e` 新增獨立 Python 標準函式庫實驗 `scripts/measure_docker_stats.py`，未修改正式採集程式。環境同下方階段量測；Python 3.9.6、Docker Engine 29.9.2（0367cbe）、API 1.56、Linux arm64，使用目前選定的 Unix endpoint，一般使用者、工具沙箱外，未用 sudo。

比較三種方式：原 CLI `stats --no-stream`、直接 API `stream=false`、同一輪內兩次 `stream=false&one-shot=true`，兩次中間等待250ms。每輪重新列出最多8個 running 容器，API 最多4個並行請求；CLI 本身會並行取得指定容器的 stats。每種方式計時前後都 inspect，要求容器 ID、StartedAt、Running、Restarting 一致。計時不包含 inventory、前後 inspect 或初始化；結果不是完整 reswho 刷新耗時。

| 完整比較 | 容器數 | CLI | 一般 API | 兩次 one-shot | one-shot 實際取樣間隔 |
| --- | ---: | ---: | ---: | ---: | ---: |
| A | 6 | 2,027.1ms | 4,002.5ms | 261.3ms | 256.1～256.3ms |
| B | 5 | 2,024.6ms | 4,004.5ms | 259.8ms | 255.1～255.7ms |

兩次完整比較的 CPU 計數皆可計算、API 記憶體欄位齊全、前後身份／狀態檢查通過。一般 API 的 CPU 取樣間隔約1秒；one-shot 第一筆皆沒有可用的 previous CPU，因此必須自己取得第二筆再算差值。有效 one-shot 樣本的回應接收時間減 Engine read 時間約1.1～2.1ms；兩者跨 host／VM 時鐘，僅供診斷，不能當作精確傳輸延遲。

這個小樣本中，one-shot 配對的 stats 階段比 CLI 約少87%耗時。一般 API 最多4並行，而容器數為5／6，形成兩批等待，故約4秒；這不是相同並行度下的 CLI／API 效能比較，也不能解讀為 API 固定比 CLI 慢。加速關鍵是取樣方式，並非只移除 CLI 啟動成本。

原計畫為三輪輪替模式順序，未完整完成，不宣稱三輪基準通過：

- 初次嘗試完成 CLI（4容器、2,035.7ms）後，因 Python 3.9 無法解析奈秒時間戳中止；已改為微秒精度解析並用合成時間戳檢查。
- 修正後取得完整比較 A，後續發生 `RuntimeError`；當時僅記錄類型，不能確定原因。
- 增加不含外部資料的錯誤原因後取得完整比較 B；第二輪一般 API 為3,726.1ms，隨後 one-shot 為305.7ms，但身份／狀態改變且僅4／5筆 CPU 與記憶體有效，因此中止，該筆不列入加速比較。停止中的空 stats 曾帶零時間戳，最後將這類無效樣本排除於時間範圍摘要；此摘要修改僅做離線檢查，未重跑 live 量測。

來源與解釋：Docker CLI [v29.9.2 stats_helpers.go](https://github.com/docker/cli/blob/v29.9.2/cli/command/container/stats_helpers.go) 的 no-stream 會要求 previous sample，並以容器 CPU 差值／system CPU 差值 × online CPUs 計算百分比。上游 Moby [client](https://github.com/moby/moby/blob/master/client/container_stats.go) 說明 previous sample 需間隔1秒；[daemon](https://github.com/moby/moby/blob/master/daemon/stats.go) 的 one-shot 分支直接讀取當次 counters。Engine 原始碼引用為研究當日的 master，未取得部署 build 對應來源；部署版本的實際行為以上述 API 量測為依據。

建議下一步先做本機 Unix endpoint 的直接 stats API 原型：每輪重新取兩筆 counters，沿用設定的 sample 間隔，限制並行與總時限，保留前後身份檢查、取消與部分資料診斷；不跨刷新保留 counters 或 metadata。需沿用 Docker CLI 的記憶體計算口徑（扣除 cgroup inactive file），目前實驗只驗證 raw usage 存在。較短取樣窗會更敏感，並不保證 CPU 值與一秒平均相同。兩次 HTTP 的 daemon／VM CPU 成本、較多容器、paused／重啟情境、SSH／TLS context、舊 Engine 與 API 協商仍需驗證；不支援的端點可保留 CLI 路徑。Podman／Apple container 尚未量測，不能套用本結果。

每次執行總預算60秒、CLI 最多10秒、socket I/O 最多5秒且受總時限限制；未啟停服務、未留跨輪資料。CLI 由 finally wait 回收、pipes 關閉，API sockets／responses 及 executor 在離開 scope 時回收。只保留匿名摘要。語法、CPU 差值邊界與時間戳離線檢查通過；Rust 正式程式未改，未重跑 Rust suite。`target/` 約3.1GiB，保留供下階段開發。

## Docker 階段量測

2026-11-02，rw-21，採集程式基於 `24a8f8a`，僅增加 ignored 測試量測入口，未改正式採集流程。macOS 26.5.2 arm64、Docker CLI 29.9.2（build a82e528）、本機 debug test build，一般使用者、工具沙箱外、未用 sudo；14個容器／5 active，三輪串行，每輪重新查詢。建置在量測前完成，總測量6.78秒，預算40秒。三輪 inventory 完整、未回報採集診斷；這不代表原子快照。

| 階段 | 第1輪 | 第2輪 | 第3輪 |
| --- | ---: | ---: | ---: |
| context show | 22.0ms | 22.2ms | 22.5ms |
| context inspect | 22.2ms | 23.7ms | 23.7ms |
| daemon info | 445.0ms | 103.5ms | 98.3ms |
| inventory | 69.0ms | 64.7ms | 66.2ms |
| inspect before（14容器） | 55.6ms | 47.3ms | 43.7ms |
| stats（5 active） | 2,054.5ms | 1,699.3ms | 1,711.8ms |
| inspect after（5 active） | 32.6ms | 32.9ms | 38.6ms |
| 命令以外的解析／歸屬等工作 | 3.93ms | 0.86ms | 0.85ms |
| Docker 採集合計 | 2,705.0ms | 1,994.5ms | 2,005.7ms |
| 採集後建樹 | 0.378ms | 0.261ms | 0.257ms |
| `docker --version` 參考（不含在採集合計內） | 23.3ms | 24.5ms | 21.6ms |

stats 佔各輪採集約76.0%、85.2%、85.4%，是這批容器下的主要耗時。前後 inspect 合計約80～88ms。第一輪 info 較慢，但只有三次樣本，不能確定原因或稱為固定冷啟動成本。

各命令耗時包含 spawn、CLI、daemon往返、讀取輸出與既有10ms polling，並非純 API 延遲。`--version` 不接觸 daemon，提供本機 CLI 啟動與本地處理成本參考，也包含 polling。命令以外時間含解析與歸屬分類，不能將這些細項再拆分。數值為小型診斷樣本，不是跨機器基準；尚未量測 Podman／Apple container。

下一步應驗證 stats 的取樣／等待機制，才能評估直接 API 是否減少等待；目前量測尚未分開 daemon 取樣與 CLI 等待。主機／各 runtime 有限併行可重疊獨立等待，但仍需另行驗證負載與取消行為。此階段沒有實作優化、快取或跨輪使用量歷史。

重現指令與資源回收規則見 [TESTING.md](../TESTING.md)。輸出僅包含匿名時間、計數與診斷代碼，容器明細僅在記憶體處理；自建 Docker CLI 子程序由原 runner wait 回收，pipe 隨 scope 關閉。建置快取沿用本專案 target，供後續開發保留。

## 無快取版本的單次快照

2026-11-02，rw-20 共用排序改動，macOS 本機 debug build、一般使用者且在工具沙箱外執行（未用 sudo），`--md --sort cpu --depth 2`，預設250ms取樣。主機與含 Docker 各量一次，依序執行，無並行建置。輸出留在記憶體並量測 UTF-8 bytes，未保存程序明細。

| 範圍 | 本輪採集 | 指令總耗時 | 輸出大小 | 行數 |
| --- | ---: | ---: | ---: | ---: |
| `--no-containers` | 314ms | 327ms | 21,378 bytes | 434 |
| 預設含 Docker | 3,018ms | 3,028ms | 21,743 bytes | 442 |

兩次主機清單皆為794個程序，採樣階段262／263ms。含 Docker 的該次輸出有一則診斷，量測當下未保留其文字，因此不能確認該次全部資料完整。隨後一次針對容器的唯讀查詢為2,970ms，回傳13個容器／4 active，該範圍無診斷；此結果不能回溯證明前次診斷原因。

這是目前成本的單次記錄，沒有前後效能比較，也不是跨機器保證。Docker metadata 每輪重新查詢；主要時間差落在包含容器查詢的採集路徑。單次輸出仍可能有數百行，`--from` 與 `--depth` 可縮小閱讀範圍，但目前不會因此跳過全機採集。

自建量測程序均已 wait 回收，pipe 已關閉；未啟停既有容器或服務。`target/` 約3.1GiB，保留供 reswho 下階段開發重用，階段邊界再檢查。

## 歷史 TUI 量測

以下為 `da90e9e` 加入重畫優化與 Docker metadata 快取時的歷史量測。後續依使用者要求移除 metadata 快取，保留事件式重畫；表內「修改後」數字不代表移除快取後的版本，尚未重新量測。

2026-11-01，macOS 本機 debug build，`target/debug/reswho --tui`，120×30 PTY、預設 depth 2、250ms sample。只看主機與含 Docker 各執行 12 秒，期間不操作；使用 `wait4` 的 user＋system CPU time 除以牆鐘時間，單一核心為 100%。計入已回收的 CLI 子程序，不含 Docker daemon／VM 的服務端成本。

| 模式 | 修改前平均 CPU | 修改後平均 CPU |
| --- | ---: | ---: |
| `--no-containers` | 9.65% | 3.21% |
| 預設含 Docker | 18.58% | 13.98% |

修改前為 `4e508ed`；修改後為本次事件重畫與 metadata 快取實作。每個數值是一段短觀察，包含首輪冷查詢；當時基線為 723 個程序、13 個容器／4 active。工作負載可隨時改變，這不是統計基準或 release 效能保證。

修改前 3 次單輪採集：主機 300–309ms（含250ms等待），含 Docker 1995–2981ms。此次沒有重新測量每輪耗時，也不宣稱延遲同比降低；刷新仍為完成採集後等待兩秒。TUI 避免無變化時每100ms重畫，Docker 快取命中且無新增容器時省去一輪完整 inspect；context、daemon、inventory、stats 與動態 inspect 仍執行。

驗證：66 項一般測試、fmt、Clippy 通過；PTY 的自動模式、鍵盤／resize、無效 scope、q／Ctrl-C、terminal 還原通過。Docker 快取測試包含30秒到期、endpoint／daemon 切換、新增與移除、啟動／重啟／停止、stats／inspect／inventory 失敗。這次沒有重跑未變更的原生 process/socket live tests。

量測程序已 wait/reap，PTY 與匿名暫存檔已關閉，未啟停既有容器或服務。`target/` 約2.0GiB，由 reswho 保留供下一階段重用，下一階段邊界再檢查。
