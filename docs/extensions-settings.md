# Extension 與設定契約

Tenqo：rw-27。

狀態：階段1／2／3已驗收。階段4的 rw-34 native-binary-only 與 rw-33 host-death lifecycle protection 已完成開發驗證，待獨立 review。rw-31 discovery／in-app diagnostics 與 rw-32 profiling 留待下一階段。

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

目前：階段4由單一開發 owner 實作，不派子代理、不寫 Tenqo，本地提交後停在 leader 指派的獨立 review 邊界。使用明確 manifest install 與既有 config root；未實作自動 drop-in discovery、marketplace、設定 editor 或 multiselect。target 保留開發 cache；測試僅用 isolated state/config，沒有修改真正使用者設定或啟動服務。

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

離線清單依固定順序列出 `docker`、`apple`、`app_bundle`、`command`；各有 id、version、protocol_version、source、platforms、capabilities、enabled、availability。`source=builtin`；`enabled` 代表平台與本次設定允許執行，與來源是否可用分開。`--no-containers` 停用本次 container collect；不影響 classify／details。清單不執行 CLI、服務或資源查詢，因此 `availability=unknown`，不宣稱服務正常。

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
