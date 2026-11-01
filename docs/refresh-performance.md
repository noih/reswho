# 刷新成本

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
