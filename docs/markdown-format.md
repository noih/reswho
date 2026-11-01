# Markdown 輸出格式

已實作：`reswho-md/8`。Tenqo：rw-8。JSON 維持 `reswho/5`，分類、ref 與資源計數不因版面調整而改變。

## 原則

固定位置與單位已能解釋的資料，使用一般文字。符號只負責區分結構、補充狀態或保護字面值；不增加純裝飾。名稱完整保留，不用 `=` 取代。格式應同時適合直接讀原始文字與 Markdown renderer。

## 文件結構

| 格式 | 用途 | 範圍 |
| --- | --- | --- |
| `# reswho` | 報告標題 | 每份一次 |
| 一般段落 | 格式版本、help 入口、時間、整機摘要與選取根節點摘要 | 不當成資源子節點 |
| `## Applications` 等 | 本次根節點下的大分類 | 只有 category 使用標題；完整保留該分類 ref／totals 摘要 |
| 巢狀 `-` | 群組與程序 | 依資源樹縮排；同一串清單不混用 `+`／`*` |
| 縮排 `>` | 所屬節點的路徑、證據、診斷 | reswho 的附註約定，Markdown 本義是引用；不佔深度 |
| 行內 code | 路徑等易被 Markdown 誤讀的字面值 | 名稱、ref、數值不額外包裝 |
| 表格／程式碼區塊 | 欄位對照／指令範例 | 放在 `--help-format`，資源樹不改成表格 |

大分類標題後用一般段落保留 `ref · CPU · RSS · oldest uptime · processes`。報告中的選取根節點也保留摘要，避免改用標題後遺失 ref 或 totals。根節點摘要不再包一層清單；其後才列子項。

粗體、斜體、分隔線、圖片、連結不預設加入。只有確實有對應內容才使用，例如 help 中的文件連結。`!` 並非圖片語法，`[bin]` 並非連結，兩者都是 reswho 約定；不使用只有特定 renderer 支援的 alert 語法。

## 固定欄位與符號

程序：`name · pid ref · CPU · RSS · uptime · ports`

群組：`name [grouping] · group ref · CPU sum · RSS sum · oldest uptime · processes`

| 標記 | 出現位置 | 唯一意義 |
| --- | --- | --- |
| `[app]`、`[repo]`、`[checkout]`、`[project]`、`[bin]`、`[system]`、`[unknown]` | 群組名稱後 | 分組方式；大分類不重複標記 |
| `?` | 對應數值欄位 | 不可取得；不能當成零 |
| `-` | 欄位內 | 已確認沒有；行首的 `- ` 則是清單語法 |
| `>=` | 部分總量／oldest 數值前 | 只知道下界；保留 unknown 數量 |
| `+ ?` | 已有部分 ports 資料後 | 其他 socket 資料未知；沿用既有格式 |
| `▸` | 資源列最後 | 還有子節點未顯示；不追加重複展開指令 |
| `!` | 診斷附註開頭 | 確實發生的採集錯誤或限制，後面必須有原因 |
| `~` | 推論附註開頭 | 此項歸屬來自推論，必須指出來源 ref 與條件；不表示數值約等於 |

`!` 不因 CPU 低、uptime 長、沒有連線或未知歸屬而出現。尚未實作的功能與一般限制放在 help，不每次輸出整段提醒。當次錯誤、實際覆蓋缺口仍顯示；不能為了精簡隱藏失敗。

`~` 限於明確的推論證據，不把含直接與推論成員的整個群組標為推論。一般樹仍只顯示程序摘要；推論來源在指定 PID 詳情顯示，避免每個 PID 都多出一行。

相同原因且同一範圍的診斷可彙總一次。不能把所有 unreadable 都稱為 permission denied：原因未確認時只報 unreadable／unavailable。PID 詳情保留原始錯誤；沒有可靠原因分類時，保留原診斷，不靠訊息字串猜原因。

## 呈現範例

以下為虛構片段，只示範層級與語意。

```markdown
## Unclassified

group:unclassified · 0.0% · 3.0MiB · oldest 5h · 2 processes

- zsh [bin] · group:59833aa5a5a1a836 · 0.0% · 3.0MiB · oldest 5h · 2 processes
    > exe `/bin/zsh`
    - zsh · pid:75554 · 0.0% · 1.5MiB · 5h · -
    - zsh · pid:75555 · 0.0% · 1.5MiB · 5h · -
```

診斷與推論附註的示意，僅在有該項證據時出現：

```markdown
> ! swap unavailable · Operation not permitted (os error 1)

> ~ project from pid:1234 · same UID/cwd · compatible start times
```

單一程序直接列 PID，不重建同名 bin 外層。`Unreadable · N processes` 保留合併、無 ref、不可展開的既有行為；不因新增符號而逐筆列出錯誤。

## 查詢與相容性

- `--depth` 仍按資源樹的邊計算，選取根為 0；標題、段落、引用與空行都不計入。全機 root → category → group → process 仍是 0 → 1 → 2 → 3。
- 全機 `--depth 0` 只有 root 摘要與必要診斷；`--depth 1` 可見分類標題及摘要，分類有未展開成員時摘要末尾保留 `▸`。
- `--from group:…` 只顯示選取分支；不為版面補回上層 category。`--from pid:…` 顯示該 PID 的摘要與詳情，子程序仍為摘要。
- 群組名稱、引用與子程序連續排列；同層群組之間留一個空行，連續 PID 不加空行。標題、根摘要與獨立附註仍用空行分段。清單內引用／子清單一致使用四空格縮排；連續附註使用 Markdown hard break。
- 格式說明維持單一入口。表格定義欄位順序及符號，另附一次 `--from`／`--depth` 指令範例，不在每個節點重複。

## 實作與驗證

1. Renderer 將 category 顯示成標題、根摘要顯示成段落；資源層級仍由既有樹決定，限制深度仍在渲染前進行。
2. 靜態採集說明採共享常數，Markdown 不輸出這些固定說明，JSON 原 warnings 保留。其餘當次 warning、process／network unavailable 附 `!`；tree 自行產生的 metadata 失敗／父鏈異常及祖先推論 evidence 依固定前綴或完整標記識別，不猜測 OS 錯誤原因。同一附註區塊只去除完全相同訊息，不跨範圍合併。
3. `src/output/markdown_tests.rs` 使用僅限測試的 pulldown-cmark 驗證標題、引用、hard break 與清單層級；欄位與數值延用原有輸出測試。

涵蓋全機／選取群組／PID、depth 0／1／多層、單一程序、部分未知、全 unreadable、直接與推論歸屬，以及含空白／反引號／控制字元的名稱或路徑。驗證不啟停使用者程序。

參考：[Markdown Guide — Basic Syntax](https://www.markdownguide.org/basic-syntax/)。標題、清單、引用和 code 採既有 Markdown 意義；狀態符號與欄位契約由 reswho 的 `--help-format` 定義。
