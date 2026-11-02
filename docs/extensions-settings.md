# Extension 與設定契約

Tenqo：rw-27。

狀態：階段1已實作設定 registry、設定檔載入、離線查詢與 sample／refresh 接入，待獨立 review。共同 extension 執行介面、安裝與外部協定仍未實作；現有 Docker／Apple adapter 是階段2遷移對象。

## Extension 邊界

一個 extension 可提供 collect、classify、details 的任意組合：採集額外資源、提出歸屬與證據、按需取得或解析詳情。parse 是內部實作，不要求另拆一種 extension。內建 Rust 模組與外部獨立執行檔遵循相同資料契約；外部程式透過版本化 JSON 協定溝通，按批處理，不為每個 PID 啟動一次。

核心保留資源身分、去重、統計、分類衝突裁決、排序、樹狀結構、TUI／MD／JSON 呈現、脫敏及有界執行。Extension 回傳結構化資料，不直接寫 Markdown 或繪製 TUI；分類須附證據，不能只靠任意 priority 覆蓋其他歸屬，也不能重複計算已有 PID。

每個 extension 宣告穩定 id、版本、協定版本、支援平台、能力及設定需求。預期以 `reswho extensions` 列出 id、版本、內建／外部來源與能力；已安裝、已啟用、來源目前可用是不同狀態。列清單不代表要執行所有資源採集。

外部程式具有執行使用者的權限；唯讀宣告、逾時與輸出限制不等於 OS 沙箱。錯誤不阻斷其他來源，但內建模組也須遵守相同期限、取消與資料驗證規則。

## 設定所有權

設定由核心統一註冊、驗證及解析。Extension 宣告設定定義或引用既有 key，讀取同一輪不可變的有效值，不能在採集時自行修改共用設定，也不各自載入不同設定檔。

| 範圍 | Key 範例 | 規則 |
| --- | --- | --- |
| 核心共用 | `sampling.interval_ms`、`refresh.interval_ms` | 核心先註冊，extension 引用 |
| Extension 共用 | `shared.test` | Extension 可宣告，其他 extension 可引用或以相容定義再次宣告 |
| Extension 專屬 | `extensions.docker.<option>` | Namespace 使用穩定 extension id；不可冒用其他 extension 的 namespace |

單一設定定義包含 key、值型別、單位（不適用則無）、範圍／允許值、預設值與說明。註冊時記錄來源 extension 及定義版本；預設值也必須通過自己的驗證。相同型別但不同用途應使用不同 key，不能只因名字相同就宣稱語意相同。

設定的「定義」、「使用者保存的值」與「本次有效值」分開管理。註冊預設值不代表把它寫進使用者設定檔，也不能把使用者的值改成 extension 自己的預設值。

## 註冊與同名規則

1. 核心先註冊；內建 extension 使用固定順序；外部 extension 依保存的安裝順序註冊。順序不得取決於目錄掃描、執行緒或程序完成時間。
2. Key 不存在時建立定義；存在且型別、單位、範圍／允許值一致時沿用。不同的候選預設值不覆蓋既有預設值，說明仍以原定義為準。
3. 型別、單位或限制不相容時明確報錯，不轉型、不換單位、不合併限制。保留原定義；有衝突的 extension 不啟用，不留下半套註冊結果。
4. 完成定義註冊後再解析所有引用，讓較早安裝的 extension 也能引用稍後註冊的共用 key。引用不存在或契約不相容時回報來源與原因。
5. 安裝順序須持久保存；更新與停用不重排，不清除使用者設定。已保存的共用定義不得因來源停用、卸載或不相容升級而靜默轉交其他 extension 覆寫；定義變更需明確遷移或改用新 key。

例：A 先宣告 `shared.test` 為毫秒整數、範圍1–5000、預設100；B 宣告相同契約、預設200。有效預設仍為100，A／B讀到同一值。若使用者設為300，兩者皆讀到300。C 若將相同 key 宣告成 bool，C 註冊失敗，既有定義和值不變。

## 有效值

優先順序固定為：本次明確指定的 CLI 參數 → 使用者設定 → 註冊預設值。未指定的 CLI 參數不能用 parser 預設值蓋過使用者設定；CLI 覆寫只作用於本次執行，不回寫設定檔。

驗證選出的有效值，不把錯誤值悄悄換成預設值，也不自動 clamp。核心共用設定無效時中止並指出 key／原因；專屬設定錯誤只使相應 extension 不啟用。未知 key 保留並提示，不能直接當成有效設定；停用或卸載 extension 不自動刪除其使用者資料。

使用者查詢設定時須能區分有效值來自 CLI、使用者設定或預設值，以及定義來源。敏感設定值不因診斷或列清單而原樣輸出。設定與安裝資訊的持久化不包含資源樣本，仍維持無歷史、無跨輪 stats／metadata cache。

## 首批共用設定

| Key | 預設值 | 語意 |
| --- | ---: | --- |
| `sampling.interval_ms` | 250 | 同一輪 CPU 配對取樣之間的最短等待；沿用目前 `--sample-ms` 的200–5000毫秒範圍 |
| `refresh.interval_ms` | 2000 | TUI 收到上一輪採集結果後，到下一輪自動採集的等待時間；須為正整數毫秒 |

`--sample-ms 500` 對應本次 `sampling.interval_ms=500`，主機、Docker／Apple 的配對取樣與未來引用該 key 的 Podman 都讀取相同值。不把實際樣本間隔當成固定500ms，仍以各來源的有效時間差計算 CPU。來源 API 無法控制取樣窗時須保留其實際口徑，不能宣稱已遵守同一取樣窗。

Refresh 與 sampling 是不同用途。保持目前同時最多一輪採集、不補追趕積欠輪次的行為；暫停停止自動排程，手動刷新仍可執行一次。一次性 MD／JSON 不因 refresh 設定而持續執行。採集 deadline 與這兩個間隔也不是同一個設定。

## 落地與驗收

先實作型別化設定 registry 與有效值解析，接入現有 sample／refresh 來源；再讓 Docker／Apple 遵循共同 extension 契約，加入只補充分類／詳情的範例，最後接入外部執行檔與清單。設定檔格式／位置、安裝操作、完整 wire schema 於各階段定義，不將本設計當作已發布介面。

驗收至少覆蓋：新 key 註冊、相容同名不覆寫、型別／單位／限制衝突、註冊失敗無部分寫入、引用缺失、CLI未指定與明確覆寫、非法值、安裝順序重啟後一致，以及取樣／自動刷新／暫停的獨立行為。Extension 資料需有 namespace 與版本；現有 JSON 封閉 enum 的擴充須依資料契約處理，不能讓新來源破壞舊解析器。

## 實作計畫與 Current

2026-11-03，使用者以 `/lead` 授權規劃並實作；基準 `cdb3348`，main，禁止 push。Leader 負責 Tenqo rw-27 與裁決，單一開發 owner 依序寫入同一 checkout，QA／review 在交接點執行，不並行修改來源。沿用現有 Rust 測試慣例，不在本輪導入新的 BDD runner。

| 階段 | 可用成果 | 驗收邊界 |
| --- | --- | --- |
| 1 | 使用者可載入及查看設定；sample／refresh 使用有效值 | Registry 原子註冊、相容同名沿用、不相容拒絕、引用與值驗證、CLI優先級、無設定維持現況、排程互不混淆 |
| 2 | Docker／Apple 使用共同 extension 介面，清單可見；分類／詳情可補充 | 內建移轉無用量／身分／輸出退步，分類證據與既有PID去重，按需詳情與一致呈現 |
| 3 | 開發者可用獨立程式擴充，用戶能登記並使用 | 版本化協定、持久安裝序與共用定義、跨次設定沿用、故障隔離／取消／界限、端到端外部範例與文件 |

設定檔採 JSON 以沿用既有 serde_json；`--config PATH` 明確指定，預設為 `$XDG_CONFIG_HOME/reswho/config.json`，未設 XDG_CONFIG_HOME 時為 `$HOME/.config/reswho/config.json`。預設檔案不存在使用預設值；明確指定卻不存在則報錯。設定檔使用版本欄位與 key→value 的 settings 物件。`reswho settings`／`reswho settings --json` 為不採集資源的有效值查詢，呈現來源並遮蔽敏感值；初版手動編輯設定檔，不新增設定寫入 UI 或自動寫入預設值。現有 `--sample-ms` 保留，另以 `--refresh-ms` 對應刷新等待。查詢 help／JSON Schema 仍不依賴使用者設定可讀或有效。

每階段由開發 owner 執行針對性測試，整合後依 TESTING.md 完成 fmt／Clippy／一般 Rust 測試；CLI／排程改動補受影響的 PTY 範圍。每階段一次獨立 review，修復只重驗受影響範圍。一般測試不得啟動真實容器服務，外部協定使用自有假程序／fixture；既有 native container probes 的未改動證據可沿用。

目前：階段1已接入 CLI／TUI，驗證結果見下方；停在獨立 review 邊界。階段2／3待下一次派工，尚不提供 extension 安裝。target 沿用專案開發 cache，階段結束記錄大小；自有 temp／child／PTY 由 fixture Drop／finally 回收，不建立或改寫真正使用者設定。

## 階段1可用介面

```sh
reswho settings
reswho settings --json
reswho settings --config /path/to/config.json --sample-ms 500
reswho --tui --config /path/to/config.json --refresh-ms 3000
```

設定檔範例（使用者自行保存，程式只讀）：

```json
{
  "version": 1,
  "settings": {
    "sampling.interval_ms": 500,
    "refresh.interval_ms": 3000
  }
}
```

`settings --json` 是設定報告，不是 `reswho/8` 資源快照；包含 `settings`、`unknown_keys`、`disabled_extensions`。各有效項有 `value`、`source`（cli／user／default）及 `definition`（key、value_type、unit、limits、default、description、provider、version、sensitive）。敏感 value／default 遮蔽為 `[redacted]`，敏感 allowed-values 不公開。未知 key 只列名稱並寫 stderr 提示，保存的值仍留在原設定檔，不能作為有效設定使用。JSON 語法／格式錯誤只顯示位置，不回顯檔案內容。

`settings::Registry::core` 先建核心定義；`register_extension` 按呼叫順序原子註冊，`resolve_uses`／`resolve` 在全部定義後檢查引用。相容性也要求 sensitive 一致，避免後來的候選解除既有遮蔽。專屬值或 uses 無效時，報告隔離該 extension 並移除其專屬有效值；核心／共用值錯誤中止解析。解析結果只提供不可變引用。本階段透過真 API fixtures 證明 extension 語意，沒有安裝器、持久安裝序或外部 wire 空殼。

Integer 型別是 JSON 非負整數；min／max 是含端點界限，allowed 是允許值集合。sample 為200–5000ms；refresh 必須為正整數 u64 毫秒，且本機 `Instant::checked_add` 可安全排程，排程當下也再次 checked-add。TUI 仍使用既有100ms事件輪詢，因此極短 refresh 是最短等待要求，不是精密計時保證。收到結果（含失敗）才開始下一次等待，不追趕積欠輪次；暫停、手動刷新與詳情按需查詢沿用原規則。

驗證紀錄：focused CLI assertion 先以 `unexpected argument 'settings'` 失敗；敏感 report fixture 先捕捉 allowed-values 洩漏，已遮蔽 metadata。PTY 首次斷言未處理游標控制碼、其次誤用大寫 PAUSED，皆修正 fixture，未改產品行為。整合完成後 `cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test` 通過（123 passed、6 個既有 live／measurement ignored）；`cargo build` 與最終 `python3 tests/pty_smoke.py` 通過（原有4種流程及新增 auto／wait／pause）。Rust temp 由 tempfile guard 清理；PTY runner 自有 config temp、shell 與終端 descriptor 皆在 finally 回收，target 保留給下一階段。未修改 native collector／runtime transport，不重跑 live 或接觸 Docker／Apple 服務。


階段交接：本地 main，基準 cdb3348；沒有 push、額外 thread／branch／worktree 或 Tenqo 寫入。target 約5.0GiB（incremental 約2.1GiB、deps 約2.7GiB），由本專案保留作後續開發 cache，下次階段邊界再檢視。PTY runner 確認所有自有 child 已 wait、descriptor 已 close、temp path 已移除；未啟動容器或停止既有程序。較原估20分鐘多出的工作是敏感 metadata 的失敗斷言及 PTY fixture 診斷；驗收範圍未降低。剩餘能力為同 owner review 修復；階段2／3及獨立 review 未執行，由 leader 接續裁定與同步 rw-27。
