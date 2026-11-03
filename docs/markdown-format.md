# Markdown 輸出格式

已實作：`reswho-md/19`。Tenqo：rw-8、rw-9、rw-16。JSON `reswho/12`，語意見 [JSON 格式](json-format.md)。容器採用獨立統計範圍，詳見 [容器採集](containers.md)；ownership group ref 依 dimension／id／canonical root 決定，早期原型 refs 應從目前 snapshot 重新取得。

`reswho --help-md` 提供精簡英文閱讀指南。CLI 用法與範例見 `reswho --help`。

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
| 表格／程式碼區塊 | 欄位對照／指令範例 | 閱讀說明放在 `--help-md`，CLI 指令範例放在 `--help`；資源樹不改成表格 |

大分類標題後用一般段落保留 `ref · CPU  RSS  ⏱︎ uptime  ΣN`。全機不再顯示重複的 Resources 摘要列，程序數放在頂端 Host CPU 摘要；其他選取根仍保留摘要與 ref。System 只有一個 System services 包裝時直接列出其子項，證據／診斷併入 System 的附註。

粗體、斜體、分隔線、圖片、連結不預設加入。只有確實有對應內容才使用，例如 help 中的文件連結。`!` 並非圖片語法，`[bin]` 並非連結，兩者都是 reswho 約定；不使用只有特定 renderer 支援的 alert 語法。

## 固定欄位與符號

程序：`name pid:REF · CPU  RSS  uptime  ports`

容器：`name [container] group:REF · CPU  memory  uptime  state  published ports`。容器群組顯示 active / containers，記憶體來源與主機 RSS 不同。

群組：`name [grouping] group:REF · CPU sum  RSS sum  ⏱︎ uptime  ΣN`

頂端 Host CPU 為整機0–100%；程序、群組與容器的CPU欄以單核心100%計算，可以超過100%。主機與容器取樣／核心範圍不同，不相加；`>=` 表示部分已知值，不是整機百分比上限錯誤。

| 標記 | 出現位置 | 唯一意義 |
| --- | --- | --- |
| `[runtime]`、`[container]`、`[app]`、`[repo]`、`[checkout]`、`[project]`、`[bin]`、`[system]`、`[unknown]` | 群組名稱後 | 分組方式；大分類不重複標記 |
| `?` | 對應數值欄位 | 不可取得；不能當成零 |
| `-` | 欄位內 | 已確認沒有；inactive 容器用量欄表示不適用。行首的 `- ` 則是清單語法 |
| `⏱︎` | 群組 uptime 前 | 最長的已知成員 uptime；使用文字樣式（U+FE0E），實際外觀依終端字型 |
| `>=` | 部分總量／oldest 數值前 | 只知道下界；保留 unknown 數量 |
| `+ ?` | 已有部分 ports 資料後 | 其他 socket 資料未知；沿用既有格式 |
| `ΣN` | 主機程序數欄位 | 整個子樹的程序總數；Unreadable 為合併程序數 |
| `[+N]` | 資源列最後 | 收合的直接子節點數（可能為群組或程序）；不是所有後代或程序總數，不追加展開指令 |
| `!` | 診斷附註開頭 | 確實發生的採集錯誤或限制，後面必須有原因 |
| `~` | 推論附註開頭 | 此項歸屬來自推論，必須指出來源 ref 與條件；不表示數值約等於 |

`!` 不因 CPU 低、uptime 長、沒有連線或未知歸屬而出現。尚未實作的功能與一般限制放在 help，不每次輸出整段提醒。當次錯誤、實際覆蓋缺口仍顯示；不能為了精簡隱藏失敗。

`~` 限於明確的推論證據，不把含直接與推論成員的整個群組標為推論。一般樹仍只顯示程序摘要；推論來源在指定 PID 詳情顯示，避免每個 PID 都多出一行。

相同原因且同一範圍的診斷可彙總一次。不能把所有 unreadable 都稱為 permission denied：原因未確認時只報 unreadable／unavailable。PID 詳情保留原始錯誤；沒有可靠原因分類時，保留原診斷，不靠訊息字串猜原因。

名稱、分類標記與 ref 以單格空白相接；ref 與數值區之間以一個 `·` 分隔，數值欄位彼此以兩格空白分隔。這是原始 Markdown 的間距，渲染器可能將連續空白合併；不使用空白數量解析資料，程式處理請用 JSON。

## 呈現範例

以下為虛構片段，只示範層級與語意。

```markdown
## Unclassified

group:unclassified · 0.0%  3.0MiB  ⏱︎ 5h  Σ2

- zsh [bin] group:59833aa5a5a1a836 · 0.0%  3.0MiB  ⏱︎ 5h  Σ2
    > exe `/bin/zsh`
    - zsh pid:75554 · 0.0%  1.5MiB  5h  -
    - zsh pid:75555 · 0.0%  1.5MiB  5h  -
```

診斷與推論附註的示意，僅在有該項證據時出現：

```markdown
> ! swap unavailable · Operation not permitted (os error 1)

> ~ project from pid:1234 · same UID/cwd · compatible start times
```

單一程序直接列 PID，不重建同名 bin 外層。`Unreadable  ΣN` 保留合併、無 ref、不可展開的既有行為；不因新增符號而逐筆列出錯誤。

## 查詢與相容性

- Markdown 的 `--depth` 按精簡後的樹計算，選取根為0；標題、段落、引用與空行不計入。一般 root → category → group → process 仍是0 → 1 → 2 → 3；System services 包裝不佔深度。JSON 保留原始樹深度。
- 全機 `--depth 0` 只有整機摘要、程序數與必要診斷；尚有子項時程序數後保留 `[+N]`。`--depth 1` 可見分類標題及摘要，分類有未展開成員時摘要末尾保留 `[+N]`。
- `--from group:…` 只顯示選取分支；不為版面補回上層 category。`--from pid:…` 顯示該 PID 的摘要與詳情，子程序仍為摘要。
- 群組名稱、引用與子程序連續排列；同層群組之間留一個空行，連續 PID 不加空行。標題、根摘要與獨立附註仍用空行分段。清單內引用／子清單一致使用四空格縮排；連續附註使用 Markdown hard break。
- 格式說明維持單一入口。表格定義欄位順序及符號，`--from`／`--depth` 指令範例集中在 `--help`，不在每個節點重複。

## 實作與驗證

1. Renderer 在獨立副本中精簡 System 包裝後限制深度，category 顯示成標題，其他選取根顯示成段落；不修改原始snapshot或JSON。
2. 靜態採集說明採共享常數，Markdown 不輸出這些固定說明，JSON 原 warnings 保留。其餘當次 warning、process／network unavailable 附 `!`；tree 自行產生的 metadata 失敗／父鏈異常及祖先推論 evidence 依固定前綴或完整標記識別，不猜測 OS 錯誤原因。同一附註區塊只去除完全相同訊息，不跨範圍合併。
3. `src/output/markdown_tests.rs` 使用僅限測試的 pulldown-cmark 驗證標題、引用、hard break 與清單層級；欄位與數值延用原有輸出測試。

涵蓋全機／選取群組／PID、depth 0／1／多層、單一程序、部分未知、全 unreadable、直接與推論歸屬，以及含空白／反引號／控制字元的名稱或路徑。驗證不啟停使用者程序。

參考：[Markdown Guide — Basic Syntax](https://www.markdownguide.org/basic-syntax/)。標題、清單、引用和 code 採既有 Markdown 意義；狀態符號與欄位契約由 reswho 的 `--help-md` 定義。

## PID 命令詳情

`--md --from pid:N` 在選取 PID 的引用附註加入 `cmd:`，有明確 script／JAR 線索時加入 `entry:`。其他列維持摘要。空字串與含空白的參數以雙引號表示，這是展示格式，不是可執行的 shell command；精確參數邊界請使用 JSON argv。

`[filtered]` 表示已遮罩的參數值；遮罩範圍與原生採集限制見 [JSON 命令契約](json-format.md#指定-pid-的命令資訊)。未知選項及相對入口不作額外路徑推論。無法取得命令時仍保留程序其他資訊，並顯示 `! command...`。
