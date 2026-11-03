# Extension 與設定契約

Tenqo：rw-27。

目前實作含 typed registry、version1 config、安全保存的 Settings UI、native external 安裝／discovery、診斷／profiling、共同 tags 與 source priority、一般資源，以及 Docker／Podman／Apple collectors。JSON12／MD19／protocol1；階段驗證與 review 的歷史狀態留在下方各段，不代表目前仍缺少那些功能。

## Extension 邊界

一個 extension 可提供 collect、classify、details 的任意組合：採集額外資源、提出歸屬與證據、按需取得或解析詳情。parse 是內部實作，不要求另拆一種 extension。內建 Rust 模組與外部獨立執行檔遵循相同資料契約；外部程式透過版本化 JSON 協定溝通，按批處理，不為每個 PID 啟動一次。

核心保留資源身分、去重、統計、分類衝突裁決、排序、樹狀結構、TUI／MD／JSON 呈現、脫敏及有界執行。Extension 回傳結構化資料，不直接寫 Markdown 或繪製 TUI；分類須附有效 tag 提案與證據，由核心依使用者 `source_priority` 裁決各維度有效提案與排除，再由樹依有效 app → project → bin 選主要分組（app／project 可並存），不能由 provider 自行繞過驗證或重複計算已有 PID。

每個 extension 宣告穩定 id、版本、協定版本、支援平台、能力及設定需求。以 `reswho extensions` 列出 id、版本、內建／外部來源與能力；已安裝、已啟用、來源目前可用是不同狀態。列清單不代表要執行所有資源採集。

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
| `sampling.interval_ms` | 250 | 同一輪 CPU 配對取樣之間的最短等待；200–5000毫秒範圍 |
| `refresh.interval_ms` | 2000 | TUI 收到上一輪採集結果後，到下一輪自動採集的等待時間；須為正整數毫秒 |

`--set sampling.interval_ms=500` 對應本次 `sampling.interval_ms=500`，主機、Docker／Apple／Podman 的配對取樣都讀取相同值。不把實際樣本間隔當成固定500ms，仍以各來源的有效時間差計算 CPU。來源 API 無法控制取樣窗時須保留其實際口徑，不能宣稱已遵守同一取樣窗。

Refresh 與 sampling 是不同用途。保持目前同時最多一輪採集、不補追趕積欠輪次的行為；暫停停止自動排程，手動刷新仍可執行一次。一次性 MD／JSON 不因 refresh 設定而持續執行。採集 deadline 與這兩個間隔也不是同一個設定。

## 原始落地與驗收計畫

以下保留最初分階段計畫；現行用法見後方 Current、Settings UI 與 [extension protocol](extension-protocol.md)。

先實作型別化設定 registry 與有效值解析，接入現有 sample／refresh 來源；再讓 Docker／Apple 遵循共同 extension 契約，加入只補充分類／詳情的範例，最後接入外部執行檔與清單。設定檔格式／位置、安裝操作、完整 wire schema 於各階段定義，不將本設計當作已發布介面。

驗收至少覆蓋：新 key 註冊、相容同名不覆寫、型別／單位／限制衝突、註冊失敗無部分寫入、引用缺失、CLI未指定與明確覆寫、非法值、安裝順序重啟後一致，以及取樣／自動刷新／暫停的獨立行為。Extension 資料需有 namespace 與版本；現有 JSON 封閉 enum 的擴充須依資料契約處理，不能讓新來源破壞舊解析器。

## 實作計畫與 Current

2026-11-03，使用者以 `/lead` 授權規劃並實作；基準 `cdb3348`，main，禁止 push。Leader 負責 Tenqo rw-27 與裁決，單一開發 owner 依序寫入同一 checkout，QA／review 在交接點執行，不並行修改來源。沿用現有 Rust 測試慣例，不在本輪導入新的 BDD runner。

| 階段 | 可用成果 | 驗收邊界 |
| --- | --- | --- |
| 1 | 使用者可載入及查看設定；sample／refresh 使用有效值 | Registry 原子註冊、相容同名沿用、不相容拒絕、引用與值驗證、CLI優先級、無設定維持現況、排程互不混淆 |
| 2 | Docker／Apple 使用共同 extension 介面，清單可見；分類／詳情可補充 | 內建移轉無用量／身分／輸出退步，分類證據與既有PID去重，按需詳情與一致呈現 |
| 3 | 開發者可用獨立程式擴充，用戶能登記並使用 | 版本化協定、持久安裝序與共用定義、跨次設定沿用、故障隔離／取消／界限、端到端外部範例與文件 |

設定檔採 JSON 以沿用既有 serde_json；`--config PATH` 明確指定，預設為 `$XDG_CONFIG_HOME/reswho/config.json`，未設 XDG_CONFIG_HOME 時為 `$HOME/.config/reswho/config.json`。預設檔案不存在使用預設值；明確指定卻不存在則報錯。設定檔使用版本欄位與 key→value 的 settings 物件。`reswho settings`／`reswho settings --json` 為不採集資源的有效值查詢，呈現來源並遮蔽敏感值；TUI 主畫面按 `s` 可編輯設定草稿，Ctrl-S 儲存；CLI override 在 TUI 中鎖定，其他設定存檔不會寫入 override。查詢 help／JSON Schema 仍不依賴使用者設定可讀或有效。

每階段由開發 owner 執行針對性測試，整合後依 TESTING.md 完成 fmt／Clippy／一般 Rust 測試；CLI／排程改動補受影響的 PTY 範圍。每階段一次獨立 review，修復只重驗受影響範圍。一般測試不得啟動真實容器服務，外部協定使用自有假程序／fixture；既有 native container probes 的未改動證據可沿用。

目前：階段5由同一單一開發 owner 實作，不派子代理、不寫 Tenqo，本地提交後停在 leader 指派的獨立 review 邊界。支援固定 config root 的 drop-in discovery 與明確 manifest install；沒有 marketplace 或 multiselect；Settings popup 見下方 rw-35 Current。target 保留開發 cache；測試僅用 isolated state/config，沒有修改真正使用者設定或啟動服務。

## 階段1可用介面

```sh
reswho settings
reswho settings --json
reswho settings --config /path/to/config.json --set sampling.interval_ms=500
reswho --tui --config /path/to/config.json --set refresh.interval_ms=3000
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

`settings --json` 是設定報告，不是 `reswho/8` 資源快照；包含 `settings`、`unknown_keys`、`disabled_extensions`。各有效項有 `value`、`source`（cli／user／default）及 `definition`（key、type、unit、options、min、max、integer、default、description、provider、version、sensitive）。敏感 value／default 遮蔽為 `[redacted]`，敏感 options 不公開。未知 key 只列名稱並寫 stderr 提示，保存的值仍留在原設定檔，不能作為有效設定使用。JSON 語法／格式錯誤只顯示位置，不回顯檔案內容。

`settings::Registry::core` 先建核心定義；`register_extension` 按呼叫順序原子註冊，`resolve_uses`／`resolve` 在全部定義後檢查引用。相容性也要求 sensitive 一致，避免後來的候選解除既有遮蔽。專屬值或 uses 無效時，報告隔離該 extension 並移除其專屬有效值；核心／共用值錯誤中止解析。解析結果只提供不可變引用。本階段透過真 API fixtures 證明 extension 語意，沒有安裝器、持久安裝序或外部 wire 空殼。

Public type 為 number／string／boolean；number 支援有限負數與小數，integer 約束適用於 whole values。min／max 是含端點界限，options 是非空同型別單選集合；沒有 options 為自由輸入。sample 為200–5000ms；refresh 必須為正整數 u64 毫秒，且本機 `Instant::checked_add` 可安全排程，排程當下也再次 checked-add。TUI 仍使用既有100ms事件輪詢，因此極短 refresh 是最短等待要求，不是精密計時保證。收到結果（含失敗）才開始下一次等待，不追趕積欠輪次；暫停、手動刷新與詳情按需查詢沿用原規則。

階段1驗證：fmt／Clippy／一般 Rust suite 及 PTY 通過；review 修復 allowed-set 相容性及 CLI 跨 subcommand 格式衝突後，8 settings／2 parser／6 CLI tests 與 fmt／Clippy 通過。6bccd35 已由 leader 驗收。

## 階段2：內建 extension

```sh
reswho extensions
reswho extensions --json
reswho extensions --no-containers --json
```

目前離線 builtin 清單依序列出 `app_bundle`、`project`、`ancestor`、`bin`、`docker`、`podman`、`apple`、`command`；各有 id、version、protocol_version、source、platforms、capabilities、enabled、availability。`source=builtin`；`enabled` 代表平台與本次設定允許執行，與來源是否可用分開。`--no-containers` 停用本次 container collect；不影響 classify／details。清單不執行 CLI、服務或資源查詢，因此 `availability=unknown`，不宣稱服務正常。

共同 Rust API 是 `extensions::Extension::execute(Context, Request) -> Contribution`。Context 含同輪不可變 resolved settings、Cancellation 與 deadline；Request／Contribution 分為 collect、classify、details，dispatcher 依能力驗證並以 provider／version 包裝可序列化的 Response。設定 registry 先建 core，再依上述順序註冊；Docker／Apple 透過 uses 引用同一 `sampling.interval_ms`。`Options.settings` 以 Arc 持有一次解析結果，worker 更新 selected PID 時不重讀設定。

Docker／Apple 使用現有 transport、Runtime／Container DTO 及同輪採樣；container dispatcher 同時啟動來源，按固定順序加入核心建樹。原本10秒 container deadline 仍從 container 階段開始，不與 host 取樣共用耗時額度。在 deadline 內回傳的原生 partial／unavailable diagnostics 保留；逾時結果拒絕，不以完成呼叫視為成功；provider 失敗或重複 container identity 只使該來源 unavailable，其他來源保留。不建立跨輪快取。Apple CLI 不存在時仍沿用原本跳過採集的行為，離線清單仍保留該內建 extension。

`app_bundle` 是遷移既有分類邏輯的實際範例：批次閱讀觀測 executable，提出帶 PID＋started_unix_us 的 AppBundle path 證據。核心核對觀測身分、絕對路徑與 Contents 所屬 bundle；stale、missing／unknown identity 或不符路徑的 proposal 被拒絕並留下 `extensions.<id>.classify` diagnostic。有效 app 證據沿用原本 app 優於 cwd project 的裁決，project 與 ancestor 邏輯留在核心；proposal 不含 metrics，不新增／複製 PID。重複有效 proposal 不重算 totals。

`command` 是按需 details 的實際範例：只有 selected-PID 查詢會提出 details request，仍在原生掃描的 identity.before／after、UID 與 executable 重驗之間執行原有 KERN_PROCARGS2 查詢。DetailProposal 綁定 PID＋start；核心驗證身分與 argv 大小界限，沿用 sanitizer、entrypoint 推導及 `[filtered]` 規則，忽略未驗證的 entrypoint／redacted_indices。缺少 argv 與診斷的空詳情會被拒絕。關閉詳情後不再查詢，pause／manual refresh 與單 worker 行為保留。

snapshot 沿用既有 Attribution／ProcessCommand 與 `reswho/8` Schema；MD／JSON／TUI 使用同一份經驗證資料，沒有新增封閉 enum 或 wire 欄位。`extensions --json` 是 metadata 清單，不是資源快照。本階段的 app-bundle 證據與 command DTO 只涵蓋已遷移能力，不是完整外部擴充契約；階段3仍須支援作者自有分類證據、namespaced parsed detail fields／provenance，屆時依實際 wire 變更處理版本與 Schema。

階段2驗證：合成 providers 覆蓋平行 dispatch／有效設定、失敗與重複 identity 隔離、stale／invalid 分類、單次 PID metrics、詳情身分與 redaction、既有 JSON Schema 及三種 renderer。離線 CLI fixture 將 PATH 清空，驗證仍能列出 metadata。`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test` 均通過；其後補齊空詳情／argv 界限 fixture 與顯式 temp close，重驗5個 extension tests、7個 CLI tests 及 fmt／Clippy 通過。`cargo build`、既有 `python3 tests/pty_smoke.py` 全通過（含 selected-PID details、config sampling／auto／wait／pause）。native transport 未修改，不跑 live／measurement 或啟動容器服務。


階段2交接：同一 main，本地 commit 後停在 review 邊界；未另開 thread／branch／worktree、未 push、未寫 Tenqo。可執行範例位於 `src/extensions/tests.rs` 的 real classifier＋validated details fixture，透過共同 API 產生核心樹、selected-PID Markdown／JSON 並驗證既有 Schema；`src/tui/tests.rs` 的 extension detail fixture 驗證按需載入與 `[filtered]` 呈現。新 CLI temp 使用顯式 close；平行 collector fixture 的 scoped threads 全部 join；PTY runner 確認 child wait、descriptor close、config temp path 移除。target 約5.2GiB（incremental 約2.2GiB、deps 約2.9GiB），保留給同專案後續開發，下一階段邊界再檢視。階段3未開始，完整外部契約與安裝能力仍待派工。


## Extension 錯誤邊界

使用者要求 collect／classify／details 任一 extension 的一般錯誤或可 unwind Rust panic 只能使該 extension 失敗，不得中止 app 或污染 terminal。共同 dispatch 將 descriptor、execute 及 provider error 轉換包在同一捕捉邊界；每次只讀 metadata 一次，fallback 不再次呼叫 extension。已取得 id 時保留來源識別；metadata 本身失敗時使用本輪穩定 unidentified 序號診斷。錯誤不回顯 provider Err、panic payload 或原始 stderr。

Panic hook 只安裝一次，以 thread-local guard 遮蔽邊界內的 panic 輸出；不逐次交換 process-global hook，並將邊界外 panic 交回原 hook。此機制涵蓋由 dispatcher 執行的 extension 呼叫；不構成 OS 沙箱，無法攔截 panic=abort、native fault 或程序被外部終止。階段3的外部程序仍須由有界 runner 接住退出／stderr／逾時，回收 child。

Execute 前後都檢查完整 context（Cancellation＋deadline），晚到結果不能生效。ProcessCommand 有 argv 時，unavailable 必須為空，以符合既有 reswho/8 Schema；混合資料在 rendering 前整筆拒絕，不轉送其診斷文字。

階段2 review 修復驗證：子程序 fixtures 覆蓋三種能力的 descriptor／execute panic、fallback、健康 sibling、stderr sentinel 與邊界外普通 panic；deadline fixture 覆蓋過期 admission 與 execute 期間到期；混合詳情 fixture 驗證拒絕、無 sentinel 與 Schema-safe snapshot。fmt／Clippy、9個 extension tests、8個 command tests、TUI extension detail test、7個 CLI tests 與5個 JSON contract tests 通過。PTY 因共用 hook 涉及 worker stderr 而重驗一次，既有7種流程全部通過；terminal 還原、child wait、descriptor close 與 config temp path 移除均確認。子程序 sentinel tests 使用 Command::output 等待並回收 child，未建立額外 temp。target 約5.3GiB，保留開發 cache。未重跑 full suite／native。

階段3 checkpoint：新增 node-semver 2.2.0，使用 Version／Range 的預設 npm 語意，不啟用 loose 或隱含 prerelease。State 與 selected --config 同目錄（預設 config root），讀取不建立檔案；mutation 使用 flock、0600 temp、fsync＋rename。安裝同 id 更新／重裝保留原序號，remove 保留 tombstone 與 canonical shared definitions；forward uses 可先保存並 effective disabled。Lifecycle、版本、external protocol fixtures、Schema9 與 isolated example 的 MD／JSON／TUI 接入已驗證。


## 階段3交接與證據

Manifest／wire／developer example 與安裝、版本、state、單輪失敗隔離、bounds 見 [extension-protocol.md](extension-protocol.md)。本版採 npm node-semver 2.2.0 的標準 Range，不使用 Rust VersionReq；host.version 與 builtin versions 為實際 CARGO_PKG_VERSION，protocol_version 獨立。合法但不相容版本／平台／protocol 登記為 effective disabled，不執行或注入新 definitions；失效 uses 與明確 disabled 分開呈現。Update 保留原本明確 disabled 狀態；remove／reinstall 保留序號與 canonical 定義。

Public settings metadata 已改為 type（number／string／boolean）、非空同型別單選 options、numeric min／max／integer、必填 default 及原有 owner／description／unit／sensitive；uses 使用同樣攤平 contract 且無 default。Number 接受有限負數／小數；core intervals 仍以安全 whole unsigned 值與既有 bounds／Instant 排程驗證，不 clamp 或 round。Options 依集合比較，相容候選不覆寫 canonical 順序／default。敏感 options 隱藏，report default／value 是明確遮蔽展示值，不是可再匯入 manifest 的原始數值。沒有 array／multiselect 設定值。

驗證：整合執行 cargo fmt --check、cargo clippy --all-targets -- -D warnings 與一次 cargo test。首次 full suite 的唯一失敗是 Apple synthetic fixture 固定讀舊 Schema8；已改用共用 JSON_SCHEMA，5個 Apple fixtures 通過。其餘 lib tests 的未變動有效結果保留；後續修復／metadata 追加以 affected scope 補驗。最終 fmt／Clippy、11 settings tests、20 extension tests（含 state、wire、npm、單輪 suppression）、8 native command sanitizer tests、external TUI test、2 parser、8 CLI、6 JSON contract tests 通過。Live／measurement 維持 ignored，未執行 native live 或真實 runtime 操作。

既有 PTY runner 已擴充 isolated install example：自有 shell 產生 custom classification／parsed details，production 一次性 JSON 與 TUI 皆通過；原有 controls、resize、pause／manual refresh 與退出還原也通過。後續 public metadata 變更只改 manifests／serde contract，依裁定不為此再次跑 PTY。新舊 Schema 各驗證正確版本；Schema8 原檔保留，Current examples 使用9；自訂來源／失敗來源的完整 snapshot 也有 assertions。

Cleanup：provider children 由共用 bounded runner 的 owned process-group guard 回收，timeout／cancel fixtures 確認 owned PID 不存在；Command::output 子程序已 wait；state mutation temp rename／cleanup 及 flock handle 皆釋放，tempdir close 成功。PTY runner 確認 child wait、descriptor close、config temp path 移除。target 約6.7GiB（incremental 約2.8GiB、deps 約3.5GiB），留給同專案 review／後續開發，下次階段邊界再檢視。沒有 push、額外 thread／worktree 或 Tenqo 寫入；後續 drop-in 需求由 leader 分票。

Metadata 收尾：public adapter／manifest Schema／example 統一使用 type/options/min/max/integer；JSON numeric bounds 保留整數精度（含 u64::MAX），整數／浮點界限比較不以有損轉換代替。最後 scoped fmt／Clippy、11 settings、20 extensions、8 CLI、6 JSON tests 通過；原 state／transport／PTY 證據依未變動範圍沿用，不因 metadata 再跑 PTY。Drop-in 功能已按使用者指示延後由 leader 分票，本次沒有 discovery／storage redesign。


### 階段3 scoped review 修復

3f9010e 的兩項必修已完成，待一次 scoped recheck。Runner 使用 waitid(WNOWAIT) 偵測 direct child 退出，保留未 reap 的 PID，先清理該 child 以 setsid 建立的自有 process group，再 wait 取得原始退出狀態。正常與非零退出都不遺留已關閉 stdio 的 descendant；timeout／cancel／pipe error 同樣由 owned guard 清理。清理時 leader PID 尚未釋放，避免在 PID／PGID 重用後發送訊號；不使用全域 process census。

Context::finish 集中所有 post-dispatch capability 驗證失敗的本輪 bookkeeping：shared command details 的身分／資料、parsed fields、classification evidence、collect duplicate identity／aggregate overflow。Aggregate 另保留原 provider id，不用 runtime display name 代替。Metadata 不可得時的 fallback label 不加入其他合法 provider 的 failure key；同輪後續能力跳過、fresh context 可重試、健康 sibling 保留。

Regression 先重現正常退出的 descendant 存活與 shared details 拒絕後仍執行其他能力。修復後 fmt／Clippy、4個 runner lifecycle tests、21個 extension tests、8個 command sanitizer tests、2個 TUI detail tests 與6個 JSON contract tests 通過；其中 scope 包含 normal／nonzero descendant、timeout／cancel、各 capability validation rejection、fresh-round retry、metadata fallback id collision 與健康 sibling。測試自有 descendants 確認消失，先前失敗 regression 的 owned PID 也確認不存在；子程序／scoped threads／temp 已釋放，target 約6.7GiB保留。未改 terminal routing，依裁定沿用 full suite／PTY／native 的未受影響證據，沒有重跑或新功能工作。

## 階段4 Current（2026-11-03）

rw-34：外部入口改為 host-compatible Mach-O native binary，安裝與執行前驗證；不支援 script／interpreter payload，不安裝或嵌入 runtime。Manifest args 僅允許空；合法 binary 內部的語言來源不做推測。既有 state 中失效／script 入口 effective disabled，保留明確 enabled、順序及 canonical settings。範例改為小型 Rust executable；protocol fixtures 改用編譯的 C binary。Python 僅保留 repository test tooling。完整 packaging boundary 見 extension-protocol.md。

rw-33：內建 C supervisor 在 pre-exec 路徑使用原生 syscall，不使用 Rust allocator／locks、shell 或 daemon。先關閉非必要 FDs（含所有繼承的 lifeline writer），fork 後以 gate 建立 group 並註冊 monitor，再允許 exec。kqueue 等待 host lifeline EOF／provider exit；殺同群子孫後 reap provider，host wait supervisor。原型曾測出極短 provider 先退出的競態，已以 gate 修正。沒有外部呼叫時零 guard；最多4個 active guards。Supervisor 提早關閉 spawn error pipe，exec failure 轉為失敗 exit status；monitor 初始化失敗不允許執行。Leader 已核准空 args 與合作 group 邊界：禁止 daemonize／setsid／setpgid 逃逸／persistent helpers；不宣稱能隔離惡意 native code、外部直接殺 guard 或 OS-wide failure。

Focused 證據：23個 extension tests、7個 command tests 通過，包括原生範例分類／詳情／Schema9、格式拒絕、state replacement、正常／非零退出、取消／deadline、四個並行 provider／descendants／guards，以及 host launch SIGKILL、running SIGINT／SIGTERM／SIGABRT／SIGKILL、return SIGKILL。每輪確認記錄的自有 PID 消失，temp roots 顯式 close。launch 測試在首個 provider 記錄 PID 後終止 host，覆蓋其他並行 launch，不宣稱逐一注入每個 syscall 邊界。補充 fault-injected kqueue 初始化失敗、missing exec 與8MiB stdin backpressure 測試通過；return fixture 另確認 completed marker，PID marker 原子發布，修正後 targeted lifecycle test 通過。

最終成本：同一有界 runner＋100ms native no-op，每模式3輪、總1.18秒。無 guard 456.337／128.369／121.372ms；有 guard 141.840／141.911／142.537ms。首輪冷啟動偏差明顯，兩組中位數約相差14ms；僅3輪，不外推實際負載。Parent PID assertion 證明無 guard 模式0個新增 supervisor、guard 模式1個；三輪 guard RSS 均1,343,488 bytes（約1.28MiB，含共享頁，不等於新增實體記憶體）。沒有跑 live runtime 或 stress campaign。

整合 gate：`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test` 通過（145 library＋2 parser＋8 CLI＋6 JSON；3個 measurement／4個 live tests 維持 ignored）。`cargo build --bins --examples`、`python3 tests/pty_smoke.py` 全部通過，含安裝原生 example 的 CLI JSON／TUI details、q／Ctrl-C terminal restoration、sampling／auto／wait／pause。原有 transport／schema／settings 未改部分沿用19e229c證據。最後僅加強 lifecycle fixture 的 return assertion／原子 marker，重驗該測試及 fmt，未重跑 blanket gate。

Disposable Rust tempfile roots（native source／binary、isolated state/config、PID markers）均顯式 close；PTY runner 回報 children／descriptors 關閉與 config temp 移除。/tmp/reswho-convert-phase4.py、/tmp/reswho-phase4-guard-debug 已移除並確認不存在；早期失敗原型記錄的 PID 50015／51193／51472／51473／51474 及 PTY fixture PID 55176／55252 均確認 ESRCH，未清理其他程序。target 約6.9GiB，開發 owner 刻意保留給同專案下一階段，下一階段邊界再檢視。無 runtime／service 安裝、無真正 user config mutation、無 push／Tenqo writes。已完成本地 coherent 提交準備，獨立 reviewer 由 leader 指派。

階段4 review P2 修復：native validator 與直接共用 manifest/state reader 改為 O_NONBLOCK 開檔，再以該 FD 的 metadata 拒絕非 regular file，保留既有 symlink 語意。真實 CLI FIFO regression 先重現1秒超時；修復後 install FIFO executable／FIFO manifest 均立即拒絕，已安裝 binary 被換成 FIFO 時 list effective-disabled 且健康項目保留。23個 extension tests、9個 CLI tests、fmt／Clippy 通過；依裁決不重跑 full suite／PTY／lifecycle。Regression 的 CLI child 在超時失敗時也 kill＋wait，temp roots 均回收。

## 階段5 Current（2026-11-03）

rw-31＋rw-32 由同一 dev owner 實作，Phase4 FIFO 修復 b50f457 指定 scoped tests 通過後開始。固定 config-root/extensions/id/manifest.json discovery、origin ownership、batch 排序持久化、失效更新停用／tombstone、防 builtin 覆寫及 persistence-failure 隔離已接入。Native-only／guard runner 不變，沿用 Phase4 lifecycle 證據。Metadata/state 只在新 discovery／更新有變更時寫入；core config 不變。Legacy origin 採保守 explicit ownership，不接受同名 drop-in 自動接管。

Current report 含 read／validate／register／collect／classify／details、provider／version／result／安全 reason code／duration_ms；未呼叫／skipped=null，真實呼叫只記一次，post-validation 可將結果改為失敗。Startup 與本輪能力 phase 分開，每輪替換，不留歷史。TUI e 獨立頁保留返回位置，主頁有 issue count；MD 一般精簡錯誤、--profile 計時表；snapshot reswho/12＋Schema，保留8／9／10，離線 extension metadata array 沿用並加 report。完整使用與 lifecycle 語意見 extension-protocol.md。

Focused discovery／timing／TUI 測試已通過：排序、restart／不重寫、update／disabled、folder deletion、壞更新不執行舊 manifest、tombstone／明確 reinstall、legacy conflict／builtin／FIFO、persistence failure 保留健康來源、offline default-root 零執行、post-validation failure／取消／skip／平行結果及 e 返回 selection／scroll／failed report。使用 owned temp roots／native fixtures，顯式 close；無 user config mutation、服務或 runtime 安裝。target cache 延用，不新增隔離 target。

整合證據：fmt 通過。Clippy 首輪發現 output helper 放在 test module 後，移至 module 前後 `cargo clippy --all-targets -- -D warnings` 通過。`cargo test` 首輪149 library passed／1 failed／3 ignored，發現未識別 provider 的 fallback 顯示名稱被錯誤當真實 id，會污染 failure suppression；修為 Option id 後原 regression 與全部27個 extension tests 通過。其餘首輪已通過的 library tests 未重跑；另2個 parser、10個 CLI、6個 JSON contract tests 通過。沒有第二次 blanket suite。Schema10 範例及8／9向後驗證均通過。

`cargo build --bins --examples` 與完整 PTY 通過，含 native drop-in／壞 sibling、JSON report、TUI e 狀態頁及返回時 counter 不增加。補上 MD `--profile` CLI assertion 後，只重跑 `python3 tests/pty_smoke.py --extensions-only`，確認一次 JSON 與一次 MD 分別只各執行 classify＋details，timing／status 不額外執行 provider。所有 owned PTYs／children／config roots 已由 runner 清理並確認；前次 fixture PID 67514／67537 亦確認 ESRCH。最終 target 約7.2GiB，由本專案 dev owner 保留供 review／後續修復，下一交接邊界再檢視。無 push、Tenqo writes、nested agents、live runtime 或額外 measurement campaign。提交 coherent 本地 main 後停在獨立 review 邊界。

### 階段5 review 修復（84846be 後）

三項 required findings 已修復：

- Discovery persistence conflict／failure 原先用啟動時舊 snapshot 判斷 unchanged，會在另一 transaction 保存 disabled／removed 後仍以舊 enabled 執行。現在失敗後重讀最新 state，再依最新 ownership／tombstone／canonical definitions 重新 discovery；只允許與最新 persisted entry 相同的候選，未保存的新／更新項目維持 persistence_failed。真實 flock＋State::mutate fixture 固定 stale snapshot／writer transaction 的順序，分別覆蓋 concurrent disable 與 remove，確認只有健康未變更來源可用、新候選不啟用、canonical definitions 與最新磁碟一致。
- finish_phase 原先在 dispatch_many 等完所有 sibling／後批次後重新 started.elapsed，將別人的等待算給 fast provider。Regression 先重現1ms fast 被報約459ms。現在保留 dispatch 實際測量值，只加該 provider 的獨立核心驗證區間；collect／classify／details 及 aggregate validation 都使用自己的 marker。1／150／300ms synthetic providers 跨兩批的 oracle 以寬鬆比例確認 fast／slow／後批次可區分，不依賴精確毫秒；failed attempts 與 skipped=null 保留。
- Selected PID 最終 verify_identity 拒絕原先只清掉 external detail payload，report 仍是 success。現在同一最終核心邊界會將實際 contributing details 的成功報告改為 failed／identity_changed，保留耗時並加入本輪 failure bookkeeping，再清除 payload。Deterministic validator fixture 經真 attach_details 與最終 validation helper，確認失敗／never_invoked／健康 classify 報告不變、敏感 Err 不出現，且同輪重試被跳過；輸出符合 Schema10。

本輪限定驗證：`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、29個 extension／discovery tests、1個 snapshot final-boundary test、6個 JSON contract tests 均通過。沒有 full suite／PTY／live／measurement 重跑；UI 與 Phase4 runner 未改，原證據保留。所有新增 fixture threads 已 join、temp roots 已 close；原生協定 fixtures 由既有 bounded runner reap。target 約7.3GiB，保留給同專案 scoped review，未新增隔離 cache。提交本地 coherent repair commit 後停止修改，供這三項 finding 的一次 scoped recheck；無 nested agents／Tenqo writes／push。


## rw-35 Current：TUI Settings popup

`s` 僅從主畫面開啟 Settings；開啟與編輯只操作 draft，不觸發 provider 或寫檔。依 registry 顯示 Core／Shared／extension、目前值與來源；無效私有值造成 provider disabled 時仍可修復，Shared canonical definition 只列一次。CLI 覆寫標為 locked。Up／Down 選欄位，Enter／Space 編輯或切換 checkbox／單選 options；Left／Right 循環選項。自由 number／string 直接在選取 value cell 編輯，Ctrl-U 清空、Enter 接受。PgUp／PgDn 捲動說明、預設值與限制；編輯列固定可見。敏感值、預設值與 options 預覽皆遮蔽，新的敏感文字從空白輸入，不把 `[redacted]` 當值保存。

Ctrl-S 明確 Save；Ctrl-R Reset selected 刪除該 user override，改用 canonical default；編輯中 Esc 僅放棄目前輸入並回 browse，保留先前已接受的 draft；browse Esc 或 Cancel 放棄整份 draft 並回主頁，保留主樹 selection／scroll。未變更不寫檔。儲存使用啟動時選定的 `--config`／XDG／HOME 路徑，0600 同目錄暫存檔、fsync、atomic rename；新增目錄為0700。設定與 extension registry 的鎖分開，非阻塞 flock 避免 TUI 等待其他 saver。儲存前重讀版本1有效檔案，只合併修改 key；保留未知／未修改設定，不序列化預設值。與 baseline 同 key 不一致會拒絕；不同 key 的外部修改會合併。壞 JSON、未知格式／版本、非 regular file、hardlink、非本人檔案及不安全目錄不會被替換。明確拒絕路徑任一層 symlink 與 `..`；macOS 暫存設定請使用 `/private/tmp` 或實體路徑。錯誤提示不含設定原值，失敗保留 draft 與舊 effective settings。鎖只能協調遵守同一 flock 的 saver；手動編輯器不遵守鎖，應避免與 Save 同時寫檔。

Save 先以既有 Contract／resolved policy 驗證；JSON number 啟用既有 serde_json 的 arbitrary_precision feature，保留大整數與有限長小數，bounds／options 比較使用十進位精確順序。成功後才將新 immutable settings 交給 worker 下一輪，既有採集保留其 Arc；更新刷新等待，不新增 collector、不重載 discovery，pause 狀態不變。資源 snapshot 使用 `reswho/12`；外部 protocol 版本獨立。

開發驗證與限制：15個 settings、27個 TUI／worker、14個 extension dispatcher、2個 parser、10個 CLI、6個 Schema tests 全通過；fmt／Clippy 與正式 binary 的 `python3 tests/pty_smoke.py --settings-only` 通過。最初 PTY 斷言誤用精確 sample 毫秒及未重建的 terminal 差分文字，已改為最短取樣範圍與 SIGWINCH 重繪檢查；沒有未解決測試失敗。PTY 使用自有0700 temp root，驗證 draft／invalid／cancel／save／CLI reload／nochange／reset／conflict／malformed failure／resize／pause；不操作真實設定或服務。尚待 leader 指派獨立 review；未跑 full suite、live container 或效能量測。target 約8.5GiB（起始7.4GiB），為保留的專案 cache；剩餘磁碟約495GiB，下一階段邊界再檢視。PTY 的 child／handles 與 temp root 已驗證回收。


rw-35 review 修復（基於3fe1d46）：P1 先以真實 Response → WireResponse 重現 fractional Container CPU 的 `invalid type: map, expected f64`，原因是 arbitrary_precision 的 number map 經 internally tagged enum buffer 後不能直接交給 f64。Container 的 optional CPU 解碼改先接受 serde_json::Number，再取得有限 f64；維持原 wire／null／missing 語意，設定與 parsed detail 的精確長小數不變。回歸使用自有 native External 同時驗證12.5 CPU、失敗 sibling 隔離及健康來源後續 classify／details。已檢查其他 tag／flatten：ClassificationEvidence、Origin 僅路徑／文字；Definition／Use 的數值由既有 Contract／Number 解碼；Listing 僅序列化，沒有相同的浮點接收欄位。

P2 窄版36×10固定保留一行欄位、至少兩行說明、兩行輸入／錯誤／值來源與三行控制。Save／Reset／Esc 不再被導航文字換行擠掉；頁移依說明區高度，避免固定跳五行漏看限制，並夾住最末頁。沿用 Ratatui 現有 rendered-line-info feature 計算實際換行，不另建 UI／wrapping framework；該 API 是目前鎖定版本的 opt-in unstable API。TestBackend 與 production PTY 都檢查主要控制、初始描述、後續限制／目前值與返回描述，並保留原寬版流程。

本次原始產品回歸為P1小數 wire 與P2窄版配置／頁移。PTY 另遇到固定兩秒初始載入等待與差分文字斷言失敗；保留既有流程，改為十秒有界載入與明確 SIGWINCH 的36×10完整畫面斷言，不將 harness failure 當作產品通過。重驗範圍為新 wire fixture、15 settings、28 TUI／worker、14 dispatcher、6 Schema、fmt／Clippy，以及 settings-only PTY；未重跑不變的 lifecycle／container／full suite。原 parser／CLI 證據沿用。測試僅使用自有 temp roots／native children，target 為保留開發 cache，review closeout 再確認大小與清理。

修復驗證結果：上述受影響檢查與最後一次 settings-only production PTY 全通過；PTY child／FD／temp root 已驗證回收，沒有未解決測試失敗。target 約9.4GiB、磁碟剩餘約494GiB，保留給後續開發；未做全域 purge。開發 owner 本地提交後停寫，待獨立 scoped recheck。

## rw-36 Current：Settings／Extensions UI punch list

本輪依使用者核准的 mockup 修復既有 UI；單一開發 owner，沒有新 dependency／framework、manifest／MD／JSON／report protocol 變更，沒有 enable／disable 管理動作。只修改 presentation、輸入路由與受影響的 TestBackend／PTY 斷言。下表四項共用同一 renderer／navigation 修復，合併於一個本地 commit；SHA 見交接回報。Tenqo 由 leader 同步，本輪未寫 tracker、未 push。

| ID | 原始觀察 → 修復 | Route／state | Evidence |
| --- | --- | --- | --- |
| UI-1 | 全畫面與大空隙 → Settings 最大76欄、Extensions 最大98欄，置中且依內容定高、受 viewport 上限約束；背景 resource view 保留並 dim，返回 selection／scroll 不變 | stay／fixed | verified：30個 TUI／worker tests，160×45、100×30、36×10 actual TestBackend frames |
| UI-2 | raw-key／重複資訊表 → General／Shared／provider 分組，友善核心 labels／描述、對齊值與 unit、changed／CLI locked 標示，Tab／Shift-Tab 操作 Cancel／Save。乾淨表單沒有固定 Draft 提示；編輯資訊不重複描述，完整 key／constraints 經 PgDn 查看，refresh 編輯只顯 Minimum:1ms。敏感預覽／輸入遮蔽，draft／save／reset 與安全儲存邏輯沿用 | stay／fixed | verified：TUI input／mask／grouping／controls tests；settings-only production PTY |
| UI-3 | raw phase report → 每個 provider 一列的 Name／Version／State／Collect／Classify／Details inventory；enabled、availability 與 error 分別表達，不聲稱 Ready。以 provider ID 保留選取；Enter（UI11 起改為 d）開 metadata／來源／path／compatibility／各 phase timing／安全 reason 詳情。未執行是無 timing，沒有相加或假0ms；candidate／registry errors 另列可捲動 diagnostics | stay／fixed | verified：provider selection／refresh／failed-report tests；extensions-only production PTY（native counter 證明開啟與返回無額外執行） |
| UI-4 | 全 cyan／內容貼邊／footer 遙遠 → 一致 border、水平內距1、灰白層級、選取背景與黃色錯誤；footer 固定面板底部。長 key／value／error 於 info／details／diagnostics 依實際換行捲動，36×10 保留必要控制 | stay／fixed | verified：三尺寸 frames、invalid／disabled／failed-provider frames、PTY resize／errors |

| UI-5 | 選取描述插入選項列、切換時後續列移動 → 清單維持連續列；描述固定在按鈕下、快捷提示正上方，正常最多2列、compact最多1列，完整文字由既有 Info 捲動。footer 高度納入內容定高，保留編輯錯誤、遮蔽與36×10控制 | stay／fixed | verified：選取首／次項 y 不變、描述位置／唯一性、完整長描述可達、compact input/error／Info 回歸與 actual frames |
| UI-6 | inventory／details footer 固定2列 → 移除硬換行，使用 Ratatui Wrap／line_count 依實際內寬計算 footer 與 panel 高度；寬版1列、窄版必要換行 | stay／fixed | verified：100×30／36×10 inventory／details hints 全可見，diagnostics、metadata、selection 與 scroll 回歸 |

實際 frame 證據保留於 `/Users/noi/.bb/thread-storage/thr_sybrkuyfgf/ui-evidence`（約52KiB），由本輪 owner 保留至 leader review 結束。`settings-*`／`extensions-*` 各有160×45、100×30、36×10；另有 settings-invalid、extensions-disabled 與 extension-details。Inventory 使用 synthetic Docker collect561.0ms、App bundles classify1.2ms、Process command details8.4ms／invalid_data，另保留 broken discovery 的 invalid_manifest。這些是 actual Ratatui TestBackend buffer，不是另外製作的 mockup，也不代表真實 runtime 效能。Leader 已接受置中／compact form／table 方向，後續五點視覺回饋已納入；尚待獨立 scoped code review。

驗證：`cargo test --lib tui:: --no-fail-fast`（30 passed）、`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo build --bin reswho --example extension_example`、`python3 tests/pty_smoke.py --settings-only`、`python3 tests/pty_smoke.py --extensions-only` 全通過。初次 render checks 發現窄版輸入／錯誤互擠與空 inventory 診斷被擠掉，已修復重驗；padding 後 diagnostics heading 換行遮住錯誤，已縮短 heading 重驗。Settings PTY 原本假設第二次翻頁是 Current 值，改版後末頁是完整錯誤；移除過時重複斷言，仍驗證 constraints、主要 controls 與返回描述，完整捲動另由 TestBackend 驗證。未將失敗當作通過。

不重跑不變的 persistence／wire／runner／container checks，不跑 full suite、live containers 或 stress。測試僅使用自有 temp config、native provider fixture、shell／PTY；兩個 PTY runner 都確認 child／handles 退出與 temp root 移除，沒有真實設定／服務變更。保留專案 `target/` 開發 cache 約9.7GiB（起始9.4GiB），剩餘磁碟約494GiB，由專案後續階段邊界再檢視，未做全域清理。

rw-36 scoped review 修復（基於48ade69，UI-2／UI-4，stay／fixed）：獨立 review 發現兩個 P2。第一個觸發為36×10長 boolean label 把 checkbox 擠出列寬，而 Info 原本只依 value 本身長度決定是否顯示，因此 default=false／current=true 只能看到 default。explicit Info 現在一律顯示完整 `Value:`，與 `Key:`／`Default:` 分開，敏感值仍經同一 preview 遮蔽，正常表單不重複目前值。新增 ordinary／sensitive 長標籤回歸，實際翻頁驗證 key、default 與 current 可讀；新增 frames：`settings-longboolean-false-36x10.txt`（Value:[x]）與 `settings-longboolean-true-36x10.txt`（Value:[masked]），位於前述 evidence 目錄。

第二個觸發為 number input=0 驗證失敗後 Tab／Shift-Tab 被 accept_input early return 擋住，Cancel 無法聚焦。焦點移動現在不接受或清除 typed draft；只有 Save 執行既有驗證，invalid Save 留在 popup，Esc／Cancel 放棄。回歸驗證雙向焦點循環、Save 被擋、回到 input 保留0、Space 作文字輸入而非按鈕，以及兩條方向的 Cancel Enter 不儲存。production settings-only PTY 增加 invalid0 → Shift-Tab Save 失敗 → Tab回input → Tab Cancel退出，檔案 bytes 不變。

修復驗證：受影響 TUI／worker 的32例中31例於第一次 batched run 通過；新增長標籤 case 起初未翻回 info 頂端而漏讀 Key，補正測試頁移後該例獨立通過，合計32例的有效證據齊全。`cargo fmt --check`／`cargo clippy --all-targets -- -D warnings`、重建正式 binary 與 settings-only PTY 通過。依 leader 裁定不再另派 independent review，不重跑 Extensions PTY／persistence／wire／full suite。PTY child／handles／temp root 已驗證回收；evidence 約60KiB，target 保留約9.8GiB、磁碟剩餘約490GiB，保留期限與 owner 不變。完成本地修復提交後停寫，由 leader 檢視新增 specific frames 與關閉 checkpoint。


UI-5／UI-6 checkpoint：基準948c7e4，未重開UI-1..4決策。focused self-review 發現並修復清單末列多捲一列，以及 compact 描述新增後擠掉 input error；保留原 Info 區、編輯時減為1列，完整描述附於 explicit Info。Info 第二頁的既有測試改驗 current value，第一頁仍驗 min／max，不刪 bounds 證據。32個受影響 TUI／worker checks 的有效結果齊全，修復後僅重跑受影響案例；fmt／all-targets clippy 通過。沒有新增 dependency、source comments、PTY 或服務；未重跑不變的 persistence／wire／完整 suite／production PTY，沿用既有安全與路由證據。

本批 actual TestBackend frames 位於 `/Users/noi/.bb/thread-storage/thr_rsj5xju577/ui-evidence`（約80KiB），含100×30、36×10首／次選項、編輯錯誤、inventory／details 與既有長 boolean 遮蔽。由本批 owner 保留至 leader 檢視結束；沒有自有常駐程序／PTY 或額外 temp root，既有 fixture 使用原 guard 清理。target 由9.8GiB維持約9.8GiB並保留，未全域清理。Tenqo 依指示留給 leader 同步；僅本地提交，不 push，SHA 見交接。


UI7／UI8 Current（基於7fbbc65；本節取代上方舊交互描述）：

| ID | 觀察 → 修復 | Route／state | Evidence |
| --- | --- | --- | --- |
| UI7 | 編輯重複值與 metadata／Edit block → 直接在選取 value cell 編輯；固定 status／description／hints 區，Enter 接受目前輸入，編輯中 Esc 只放棄該輸入並保留先前 draft，browse Esc／Cancel 放棄整份 popup。Tab／Shift-Tab 保留 invalid 輸入，Cancel 可達，Save 才驗證。explicit PgDn Info 提供完整 current／default／constraints／key／描述與錯誤；CLI lock 與敏感遮蔽沿用。窄版截短 label／value，編輯保留尾端與游標 | stay／fixed | verified：TUI keyboard／render tests；100×30、36×10 actual frames；單一 settings-only production PTY 的 invalid→Cancel、兩層 Esc、Save／CLI reload |
| UI8 | Extensions 跨開 Settings／q退出與重複 Enter → 只有主畫面 s／e 開 modal，inventory→details→Esc 返回；inventory／details／process-details／help 吞掉 s／e／q，不跨開 modal 或退出。q 僅主畫面退出，Ctrl-C 不變；搜尋字元仍是文字。同步提示與既有 PTY 進出流程 | stay／fixed | verified：top-level routing、selection／scroll／pause／無新增採集要求 tests；同一 production PTY 驗證 inventory／details 的 s／q／e 隔離與兩層 Esc 返回 |

35個受影響 TUI／worker tests、`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings` 全通過。重建正式 binary 後的 `python3 tests/pty_smoke.py --settings-only` 通過，沿用既有 persistence failure／conflict／reset／resize／pause assertions，沒有刪除 oracle。初次新增 Unicode fixture 誤用不允許的 key，第二次把寬字元 TestBackend continuation cells 當文字空白；修正 fixture 與讀法後通過，實際 cursor cell 仍必須是 `_`。Clippy 的 collapsible_if 已修正。focused self-review 已檢查 masking／safe escaping、按鈕焦點與 caller 路由；沒有新 dependency、provider／wire／persistence 變更。

Actual frames：`/Users/noi/.bb/thread-storage/thr_k8t7bux9h6/ui7-8-evidence`，約88KiB。Leader 已接受 edit100×30／invalid36×10／extensions36×10 方向，並保留證據至 focused review 結束。PTY child／handles／temp root 已驗證回收；沒有修改真實設定、服務或既有程序。target 約9.9GiB，保留專案既有開發 cache，由後續階段邊界再檢視。

本輪只有一個 production PTY journey；未重跑 external native counter／extensions-only、full／backend／wire／live suites。既有 no-extra-provider oracle 保留且路由已更新，TUI state tests 驗證 modal 導航不新增採集要求。Tenqo 未寫，僅本地 commit、不 push；完成後停寫供 focused review。


UI7 scoped P2 修復（基於fe8de40；stay／fixed）：review 的37×10 synthetic extension `unit=milliseconds` 重現輸入6789只顯示`[_]`，因此前述 inline-width 證據不足以涵蓋長 unit。欄寬改先保留至少8欄 value 空間及 locked／changed marker，再截短 label／unit；unit 與 suffix 用 Ratatui terminal display width 計算，不用 UTF-8 byte 長度。production refresh／sampling definition 未修改，visible unit 維持 `ms`；milliseconds 僅為 extension robustness fixture，完整單位仍可由 explicit Info 查看。

新增37×10 regression 先紅後綠，覆蓋 core ms，以及 synthetic ms／milliseconds／秒鐘／長 Unicode unit，各 extension case 同時驗證 ordinary typed tail、實際 `_` cursor cell 與 sensitive `****` 遮蔽。9個受影響 Settings keyboard／render tests、fmt／all-targets Clippy 通過；沒有重跑整個 TUI suite／PTY／backend／wire／live gates，沿用未受影響路由與既有 production PTY 證據。未新增 framework／dependency，沒有 persistence 或 provider 變更。

新增 actual frames 與既有證據同在 parent `ui7-8-evidence`，總計約124KiB；正式 core frame 為 `settings-unit-core-masked-false-37x10.txt`，長 unit fixture 明確命名為 `settings-unit-long-masked-false-37x10.txt`，另含 Unicode 與 masked variants。由 leader 保留至檢視結束。本輪沒有建立 PTY／child／temp root，TestBackend 僅記憶體資源；target 約9.9GiB仍保留。僅本地 commit、不 push、不寫 Tenqo，完成後停寫供 leader actual-frame inspection；依裁定不另派 independent recheck。


UI9 Current（基於d1f698d；stay／fixed）：依使用者最新裁定，footer 僅留必要操作，不再列通用 Move／Scroll 或導航別名。主頁寬版顯示 Enter Toggle、/ Search、s Settings、e Extensions、? Help、q Quit 與 refresh／retry／進行中回饋；窄版保留 Help、Quit 與刷新提示。方向鍵及 hjkl 導航保留於 Help，process details 的方向鍵捲動也補入 Help。Extensions list 當時僅 Enter Details／Esc Back（UI11 起改為 d Details），extension details 僅 Esc Back；process details 保留 refresh／pause／Esc。Settings browse 移除 Move，保留 Edit／Controls／Save／Reset／Discard／Info；編輯提示維持情境。modal 背景隱藏無法作用的主頁快捷提示。

驗證：8個受影響既有 renderer／refresh／navigation tests 通過，`cargo fmt --check` 與 `git diff --check` 通過。既有 renderer assertions 擴充檢查主頁與 process details，100×30、36×10 actual TestBackend frames 均檢視；Settings 編輯、invalid、focus／description 與 Extensions diagnostics／metadata 斷言保留。初輪發現 modal 背景殘留主頁提示，修正後重驗通過。僅提示／局部 render 分支變更，按鍵路由與既有 key-behavior 證據沿用，無新依賴、設定或 wire 變更。

證據保留於 parent `/Users/noi/.bb/thread-storage/thr_k8t7bux9h6/ui9-evidence`，由 leader 檢視與決定清理。未建立 PTY／child／temp root，TestBackend 僅記憶體資源；target 約10GiB沿用專案開發 cache。依指示未跑 full suite／Clippy／PTY／backend／獨立 review，未操作真實設定、服務、Tenqo 或 push；僅本地英文 commit，tracker 與 acceptance 交由 leader。


## 分類來源優先序（rw-38）

使用者 config version1 新增頂層 `source_priority: ["app_bundle", "project", "ancestor", "bin", "example"]`，與 settings definitions、安裝序及 shared canonical owner 分開；`settings --json` 可離線檢視已保存 ID。Extensions dialog 明示 Priority · highest first，Shift-Up/Down 改 draft，Ctrl-S 保存，Esc 放棄；Right／l 開詳情、內層 Left／h 或 Esc 返回 draft 列表，s／q 不新增導航或離開程序。小終端保留快捷鍵。內建來源參與排序但不可移除。

沿用 Settings 安全 transaction：no-follow path、鎖、baseline 衝突檢查、獨立 key merge、0600 temp 與 atomic rename。Malformed／衝突／寫入錯誤保留 draft 並顯示錯誤，沒有假的 applied 狀態。成功才發佈新的 immutable Resolved；採集中保留既有 Arc，下一輪生效。缺席 ID 保留位置，新的 provider 依既有 inventory 附加底部，更新不重排；首次保存記錄完整現有順序。


UI12 Current（基於2432756；stay／fixed）：依使用者裁定，主樹 Right／l 展開收合分支、進入已展開分支的首個子列，或開啟程序／resource leaf 詳情。Left／h 先收合，否則回父層；詳情以 Left／h 或 Esc 返回原樹位置。d 保留父程序直接開詳情的用途，主 footer 改用簡短 → Open。Extensions Right／l 開詳情，Left／h 或 Esc 分兩層返回；內層保留 priority draft／選取／scroll，外層放棄 draft。Enter 不開詳情。Settings、搜尋、q／s 隔離及 Shift-Up／Down、Ctrl-S 行為保留。詳情原有折行／垂直捲動沒有水平平移；[／] 繼續捲動主樹底部路徑。

### 本次執行的通用覆寫

`--set KEY=VALUE` 可重複使用，適用 snapshot、TUI、`settings` 與離線 `extensions` 清單。優先序為 CLI > config > default。同 key 取命令列最後一次出現的值，再驗證有效值；子命令前後皆可放置。缺少 `=` 的項目一律報錯。

只在第一個 `=` 分割。string 直接使用右側原文，保留空字串、空白與其餘 `=`，不需額外 JSON 引號；number／boolean 使用 JSON scalar，再依已註冊 contract 的 type、options、min、max、integer 驗證，保留精確整數與小數。shell 含空白時仍須引用整個參數。

```sh
reswho --set sampling.interval_ms=500 settings --set refresh.interval_ms=3000 --json
reswho settings --set 'extensions.example.label=work = local'
reswho --tui --set shared.mode=fast
```

extension／shared 範例須有可用 manifest 註冊對應定義；不會為停用 extension 臆造定義或執行 provider 取得型別。`source_priority` 不是註冊設定，不能以 `--set` 修改。未知 key 或無效 CLI 值會在 collector／provider 執行前拒絕，診斷只包含 key 與原因；config 的無效 extension 值仍沿用隔離行為。敏感值在設定預覽遮蔽，來源標示為 `cli`。

`extensions install/enable/disable/remove` 在任何 mutation 前拒絕 `--set`。help／Schema 仍可離線讀取，不載入 config。舊版 `--sample-ms`／`--refresh-ms` 已移除，無 alias；JSON 的 `sample_ms` 欄位不變。

Clap global append 在跨子命令時會保留較深層的參數集合，因此 CLI 在 Clap 語法檢查後依原始 argv 收集所有 `--set`，再交由 Registry 合併與驗證。
