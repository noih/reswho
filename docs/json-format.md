# JSON 統計語意

Schema：`reswho/7`。JSON 保留原始資源樹，供程式依穩定欄位與 ref 查詢；沒有套用 TUI／Markdown 的冗餘列隱藏。

| 欄位 | 意義 |
| --- | --- |
| `system.cpu_percent` | 整機CPU使用率，0–100% |
| `process.cpu_percent` | 該程序CPU，單一主機邏輯核心為100%，可超過100% |
| `totals.known_cpu_percent` | 子樹已知程序CPU總和，單核心尺度；搭配 `cpu_unknown` 判斷是否完整 |
| `totals.known_resident_bytes` | 已知程序RSS總和；搭配 `memory_unknown`。共享頁可能重疊，不等於整機記憶體 |
| `system.memory_used_bytes` | 整機已用記憶體，與程序RSS總和是不同觀測 |
| `container_totals` | 容器獨立統計；CPU為daemon單核心尺度，memory是Docker CLI來源，不能與主機totals相加 |

`root` 是選取範圍的根，`root.totals` 是其子樹彙總，不是另一份額外用量。父節點 totals 已包含子節點，不可再將父子 totals 相加。全機 `group:all` 的 totals 只包含主機程序，排除 Containers 子樹。

未知值保留 `null` 與 unknown 計數，不補零。容器 inactive 的資源欄位也是 `null`，須連同 state 判讀；它們不計入 active 用量的 unknown。

`--depth` 計算原始樹的邊，根為0；收合只移除顯示的 children，totals 維持原始彙總，`omitted_children` 記錄省略的直接子節點數。原始 System services 節點與 ref 保留，可用 `--from` 查詢。Markdown 的 System 精簡只影響其呈現深度，不改 JSON 契約。
