# macOS 採集驗證

Tenqo：`rw-1` 至 `rw-9`。日期：2026-11-01。環境：macOS 26.5.2、Apple Silicon、10 logical CPUs、32 GiB RAM，Rust 1.99.1。

## 可執行範圍

```sh
cargo run -- --md
cargo run -- --json --depth 1
cargo run -- --md --from group:projects --depth 2
cargo run -- --md --from pid:1234 --depth all
cargo run -- --help-format
```

`pid:1234` 是範例，須替換為當前 PID。`group:` ref 可直接從輸出複製。

原型提供程序清單、兩次 CPU counter 差值、RSS、UID、PPID、啟動時間、執行時間、執行檔與工作目錄，TCP／UDP socket 端點與 TCP 狀態，以及整機 CPU／RAM／swap。沒有 daemon 或跨次執行的觀測資料庫。

支援一次性 Markdown／JSON、`--from` 與 `--depth`。非互動執行預設 Markdown；互動環境預設 TUI 的分支尚未實作，因此須明確帶 `--md` 或 `--json`。`--tui` 目前回報未實作，不會以靜態輸出冒充 TUI。

已加入 [Docker 容器 metadata／stats 與分類](containers.md)，可用 `--no-containers` 跳過。

未實作：Ratatui 畫面、容器內程序、其他 runtime adapter、命令參數、Review 規則與持續觀測。不根據本次取樣產生遺留判定。

## 分類與計數

- 分類不使用 app／工具名稱清單，也不假設安裝於 `/Applications`。
- 執行檔路徑符合最外層 `.app/Contents/` 或 `.app.bundle/Contents/` 結構時歸入應用，涵蓋巢狀 helper 與 clone bundle；各路徑保留獨立子群組，依共同 bundle ID 建立應用總量，不按同名合併。這是路徑與宣告 metadata 證據，不是簽章驗證。
- 其他程序從 canonical cwd 向上找最近的專案描述檔或 `.git` 邊界，歸入 `Projects`。若應用另有 cwd 專案線索，保留為 candidate，主分類仍為應用。
- 系統執行檔依系統目錄結構保留於 `System`／`System services`；其餘保留 `Unclassified`，兩者都可再依 bin 彙總。
- 無直接歸屬時，可沿父程序尋找應用／專案，但每一步須有相同的已知 UID、相同的已知 cwd，且父程序啟動時間不晚於子程序。不同或未知 cwd、PID 1、身分不全、循環與超過 128 層均中止推論。直接 app／專案線索不被祖先覆蓋。
- 推論在程序 evidence 明示來源 PID 與條件；可讀 PPID 不等於已確認同專案。保守規則可能漏分，不為降低未知數而硬猜。
- 移除 Docker 名稱特例，runtime adapter 尚未提供 metadata 前，依一般應用／系統路徑分類，不能視為完整容器用量。
- 同一程序只屬一個分類。不同分類的 PPID 保留，但不把程序複製到另一組。
- 同一組內的父子關係須有相容的啟動時間；循環或超過 128 層的異常鏈轉為明示的獨立根，避免遺失或重複程序。
- group ID 使用分類與來源 key 的 SHA-256 前 16 hex；同一快照會檢查碰撞並拒絕混組。識別規則屬原型契約，未來若改變必須更新格式版本。
- 深度從選取起點 0 計算，預設 2；metadata 不算資源階層。限制深度不改變已計算的 totals。
- Markdown 收合標記為原列尾端的 `▸`，不另列展開指令；用法見 `--help-format`。此標記僅影響顯示，ref 與 JSON omitted_children 不變。
- 路徑證據簡寫為 `app PATH`／`git PATH`，意義集中在 `--help-format`，不逐項描述辨識步驟。

## Projects 與分類優先級

直接 App → 直接 Project → 符合既有 UID／cwd／啟動時間條件的祖先歸屬 → Bin → Unknown。直接觀測不被祖先推論覆蓋；System 是路徑大分類，保留其摘要層。分類不代表用途、閒置或可安全關閉。

`Projects` 取代 `Development`，分類 ref 改為 `group:projects`。專案標記只檢查 regular file 存在，不解析／執行內容、不需要安裝對應工具；它是目錄線索，並非確認專案有效或正在開發。名稱相同仍依 canonical 完整路徑分開。

| 專案／建置工具 | 標記 |
| --- | --- |
| Rust、Node、Python | `Cargo.toml`、`package.json`、`pyproject.toml` |
| Maven、Gradle／Kotlin、Scala | `pom.xml`、`settings.gradle.kts`、`settings.gradle`、`build.gradle.kts`、`build.gradle`、`build.sbt` |
| Go | `go.work`、`go.mod` |
| Ruby、PHP、Dart／Flutter、Elixir、Swift | `Gemfile`、`composer.json`、`pubspec.yaml`、`mix.exs`、`Package.swift` |
| Deno、CMake、Meson、Zig | `deno.json`、`deno.jsonc`、`CMakeLists.txt`、`meson.build`、`build.zig` |

遇到最近 `.git` 邊界後不再向外搜尋，避免把內層 repository 認作外層專案。沒有 Git 時採最近標記；沒有標記、cwd 不存在或無法解析時不宣稱為專案，cwd 仍保留在 JSON 與 PID 詳情。相同目錄有多個標記時按固定標記表選一個作 evidence。未解析 workspace 成員清單，不把目錄外成員自動納入。

有 Git 時保留 repository → checkout → project → process。相同位置的 checkout／project 共用一列；只有一個有程序的 checkout 時不額外建立 repository 層。只在子專案有程序時仍保留 checkout 路徑層。子專案群組是依目錄歸屬，不冒充程序父子關係；跨組 PPID 保留。原 checkout ref 沿用既有 hash namespace，非 Git／子專案 ref 使用 project namespace。

## Bin 與 Markdown 呈現

Bin 對尚未歸入 App／Project 的程序樹，依根程序完整 executable 路徑彙總，不按程序名稱猜測。子程序留在原程序樹，因此 bin totals 包含根程序及其觀測到的未分類子程序，成員不一定全是同一個執行檔。多條同路徑根程序樹合併；僅一個程序時直接顯示 PID，不建立 bin 群組。缺少 executable 的根節點留在 Unknown ownership。Bin ref 僅在該群有多個程序時存在。

群組使用 `[app]`、`[repo]`、`[checkout]`、`[project]`、`[bin]`、`[system]`、`[unknown]` 標示方式；分類層不重複標籤。選擇 A：程序名稱完整保留，不以 `=` 省略。名稱與 ref 使用一般文字，不加粗體或 ref 反引號；路徑使用行內 code，避免特殊字元被 Markdown 解讀。格式標記只用於區分語意或保護字面值：`-` 清單表示資源樹，縮排 `>` 引用表示屬性／證據／診斷，並不佔資源深度。單一 PID 的路徑與詳細資訊透過 `--from pid:N` 查詢。

```markdown
- zsh [bin] · group:59833aa5a5a1a836 · 0.0% · 31.4MiB · oldest 11d · 50 processes

    > exe `/bin/zsh`

    - zsh · pid:75554 · 0.0% · 1.5MiB · 5h · -
```

格式版本為 `reswho-md/10`／`reswho/7`。大分類以標題呈現、根與分類摘要為段落；診斷 `!`／推論 `~` 僅用於附註，詳見 [Markdown 輸出格式](markdown-format.md)。JSON Node 新增 `grouping`，群組值為 category/app/repo/checkout/project/bin/system/unknown/runtime/container，程序為 null。容器資訊使用 container、runtime 與 container_totals 欄位，與主機 totals 分開。資源與 evidence 分開，JSON 不加入 Markdown 標記。

## Repository 與 checkout

讀取 `.git` directory／gitfile 與 `commondir`，解析相對路徑後以 canonical common directory 確認同一 repository。每個 checkout 只讀取一次 metadata；文字 marker 限 8 KiB，不啟動 Git 子程序、不讀取環境變數或執行 hooks。metadata 缺損或不可讀時保留原 checkout 群組並附原因。同名獨立 clone 不合併，不依賴 BB 的目錄命名。

多個有程序的 checkout 會增加 repository 父群組，內層顯示 `main checkout`／`worktree <Git admin directory name>`，各自保留路徑、ref 與程序父子樹。只有一個 checkout 時不增加父層；沒有本次程序資料的 checkout 不列入。Repository 父群組 ref 以 common directory 識別，僅在多 checkout 時存在；原 checkout ref 不變。深度仍按實際樹邊計算，因此多 checkout 的程序會比單 checkout 深一層。

Repository totals 加總其 checkout；category／全機不再次加總父群組與子群組。新增階層與 ref 語意使用 `reswho-md/5`／`reswho/3`；原始程序與各 checkout 指標不變。

參考：[Git repository layout](https://git-scm.com/docs/gitrepository-layout)。

## Application 與 bundle

每個 bundle 路徑讀取一次 `Contents/Info.plist`（XML／binary，regular file，上限 1 MiB）。相同非空 `CFBundleIdentifier` 的多個路徑新增共同應用父群組；ID 限英數、`.`、`-`。只有單一路徑時維持原階層；metadata 缺失、無效或超限時維持路徑分組，不按名稱猜測。父群組名稱優先取 `CFBundleDisplayName`，再取 `CFBundleName` 或原路徑名稱，選擇順序固定。

父列 evidence 為 `bundle ID`；內層 `bundle VERSION` 取磁碟上的 `CFBundleShortVersionString`，缺少版本時僅顯示 `bundle`。這不是正在執行的程序版本；例如 Chrome 更新後 helper 仍可能使用舊 framework。各內層保留 `app PATH`、原 ref 與程序樹。父 ref 依 bundle ID 產生，僅多路徑時存在。不同路徑的 PPID 保留，但不複製程序；總量加總一次，深度按實際階層計算。

此變更使用 `reswho-md/6`／`reswho/4`。bundle ID 是應用自行宣告的分類線索，沒有驗證簽章，也未區分 Chrome profile、tab 或 renderer 角色。

參考：[Apple CFBundleIdentifier](https://developer.apple.com/documentation/bundleresources/information-property-list/cfbundleidentifier)、[plist Value](https://docs.rs/plist/1.10.3/plist/enum.Value.html)。

## 指標與邊界

程序 CPU 使用 native task ticks／同一程序兩次觀測的 Mach ticks 差值，100% 代表一個 logical core。整機 CPU 使用 host ticks，100% 代表整機。程序身分在觀測前後核對 start time，缺失或不同身分不計算 CPU。

RSS 為 `proc_taskinfo.pti_resident_size`，不含被換出或壓縮部分。群組總和仍可能重複包含不同程序共用的頁，不能當成唯一實體用量。JSON totals 的 `known_*` 只加總可取得的值，並附 unknown counts；Markdown 用 `>=` 表示部分總和。

整機 RAM used 以 `(internal - purgeable + wired + compressor) × page size` 計算，與 RSS 群組總和不同。Mach VM statistics 只要求本次使用的欄位已由 kernel 回傳，不能要求整個最新版 struct 長度，否則不同 ABI revision 會被誤認為不可讀。

FFI buffer 使用 libc／libproc 的固定 C struct，以實際回傳長度驗證欄位可用性；字串由有界陣列讀取。Mach host port 在查詢後釋放。不讀取環境變數或命令參數。Markdown 路徑／名稱轉義控制字元、換行、方向控制字元與結構符號。

參考：[Apple proc_info.h](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/sys/proc_info.h)、[sysinfo macOS counter 實作](https://docs.rs/sysinfo/0.40.1/src/sysinfo/unix/apple/macos/process.rs.html)。計數器直接使用 libc／mach2，socket struct 使用 libproc，沒有 sysinfo 相依。

Markdown 在群組 RAM 後顯示 `oldest`：組內最長的已知 uptime，包含閒置時間；未知成員存在時加 `>=`，全部未知為 `?`。數值在限制深度前計算，收合不影響結果。JSON `reswho/2` 的 totals 加入 `oldest_uptime_seconds` 與 `uptime_unknown`，既有欄位保留。

目前 Markdown 為 `reswho-md/10`。一般樹狀展開只顯示程序摘要；`--from pid:N` 顯示選取 PID 的詳細資訊，其子程序仍為摘要。完全無可讀身分、路徑或指標的程序合併成 `Unreadable · N processes`，不附 ref／展開箭頭；部分可讀程序不折疊。此為 Markdown 呈現規則，不改變分類與 totals；JSON 保留個別程序，明確 PID 查詢仍可讀取診斷原因。

## Socket 採集與格式

透過 `proc_pidinfo(PROC_PIDLISTFDS)` 與 `proc_pidfdinfo` 讀取 TCP／UDP IPv4／IPv6 socket，使用 libproc 的 C struct，核對完整回傳長度後才解碼。FD inventory 最多重試 3 次、上限 65,536 筆；超過範圍回報不可用。libproc 的建置需要 macOS SDK／clang。

同一程序內依 OS socket identity 去除重複 descriptor，不跨程序加總連線數。對外只呈現 FD、protocol、local／remote endpoint、TCP state，不輸出 kernel pointer。FD 消失、權限或 ABI／IP metadata 不完整時保留部分結果及錯誤。整個程序身分在採集前後核對；變更或不可確認時丟棄觀測，保留原始 OS 錯誤。

- `network: null`：無法取得可信的 socket inventory。
- `network.complete: false`：僅取得部分已列出的 descriptor，附 `unavailable` 原因；`true` 只代表本次列出的 FD 查詢完成，非原子快照或活動歷史。
- Markdown ports：`tcp:8080` 為監聽，`udp:5353` 為 bound UDP port（不代表服務端）。已建立 TCP 連線另列 endpoint／state。
- `-` 表示本次成功採集範圍內沒有符合項目，`?` 表示未知，`tcp:8080 + ?` 表示已知項目加未知部分。零連線不表示可以停止。
- 尚未收集 Unix socket、流量或無活動時長。

rw-2 當時格式更新為 `reswho-md/2`／`reswho/2`（目前 Markdown 為 `/8`）：新增 network 資料與逐程序歸屬證據；移除名稱特例，因此舊 `group:containers`／Unknown project 分組可能不存在。hash 演算法不變，沒有提供舊 group ref alias；應從新快照複製 ref。

參考：[libproc socket types](https://docs.rs/libproc/0.14.11/libproc/net_info/index.html)、[Apple proc_info.h](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/sys/proc_info.h)。

## rw-9 容器驗證

後續修正 C1／C2：Markdown `/10`、JSON `/7` 將 inactive 排除於用量與 uptime 的 unknown 計數，增加 active／inactive／activity_unknown 計數，Compose 附註合為單行。52 個一般測試、fmt、clippy 通過；回歸測試先重現 exited 造成 unknown，再驗證 mixed／全 inactive／狀態未知／轉換中／部分 inventory。真機 JSON 為 14 容器、5 active、9 inactive，三項 unknown 計數均為 0，約 3024ms；Markdown 顯示 `5 active / 14 containers`，已停止列為 `- · - · -`。兩次快照用量不同屬正常取樣差異。

C3 唯讀調查：當時未分類的 bf9 容器為 PostgreSQL image，僅有自訂 `owner=bf9-a1`、Docker volume，缺 Compose metadata、working directory 與本機 bind 路徑，因此保留容器層級；不把該 label 或名稱前綴當作專案路徑。未啟停既有服務或容器。測試自有程序已回收；`target/` 約 1.4 GiB，沿用本專案開發快取，下階段再檢視。

`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、49 個一般測試通過。容器測試涵蓋 context／DOCKER_HOST 優先順序、遠端路徑隔離、Compose／bind 推論與候選衝突、缺 CLI／缺欄位／停止與消失／重啟、inventory 上限、零與未知、端點身分、獨立總計、Markdown parser 與深入查詢。超時測試以自有 shell exec sleep 驗證 partial output、kill／wait 及 PID 不再存在。

最終真機唯讀 JSON 快照為 735 個主機程序、15 個容器（6 running，6 份可用 memory），整輪採集約 3039ms。程式核對主機 RSS／程序數只來自主機節點，容器數／memory 只來自 container 節點，所有 refs 唯一；選取一個容器後以 `--depth 0` 成功讀取 Markdown 詳情。先前快照為 13 個容器／4 running，期間環境自行變動，非固定測試資料或效能保證。

另用自有暫存目錄下不存在的 Unix socket 作唯讀失敗檢查，Docker unavailable 不阻斷主機快照，inventory 不假報 0。暫存目錄已移除，測試自有短命程序已回收；沒有建立、停止或刪除容器。未重跑未修改的 native process／socket live tests，沿用既有 native 採集證據。`target/` 約 1.3 GiB，為本專案後續開發保留，下一階段再檢視。

## rw-8 Markdown 語意格式驗證

`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、34 項一般測試與 process live test 通過。新增四項 parser／格式情境，涵蓋 category headings、清單父子深度、附註引用與 hard break、全機／群組／PID 選取、depth 0／1／多層、部分未知與全 unreadable，以及特殊名稱／路徑不注入標題、清單、連結或 HTML。

`!` 與 `~` 依實際 warning／unavailable 與程式明示的 evidence 標記產生；測試確認低 CPU、長 uptime 不加異常標記，process exited 不被改寫成權限錯誤。靜態說明採共享常數，JSON 原 warnings、schema、分類、ref 與 totals 維持原規則，Markdown 版本升為 `/8`。

真機 depth 1 輸出顯示四個大分類標題、各自 ref／totals／收合標記，以及當次 host swap 不可用診斷；採集約 343ms（含取樣等待），非效能保證。Live fixture 的自有 sleep 與 TempDir 均已回收；未重跑未變動的 socket live test。建置快取保留供本專案下一階段使用。Parser 僅為 dev dependency，不加入執行檔的功能相依。

## rw-6／rw-7 分類與格式驗證

`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、30 項一般測試與 process live test 通過。專案測試涵蓋全部 21 種標記、同名不同路徑、最近標記／Git 邊界、app 優先、checkout 子專案、部分未知指標與收合後 totals。Bin 測試涵蓋保留子程序樹、不同執行檔分開、app／project 不被移走、System 摘要層、單一程序不建群組。格式測試涵蓋分類標籤、完整名稱、引用屬性、code span 內反引號／底線／控制字元與既有 Unreadable 行為。

Live fixture 先以自有 `.git` 分類同一個 sleep，再換成 `go.mod`，確認沒有 Git 仍在 Projects；自有程序已 kill／wait，TempDir 移除並確認不存在。未執行 socket live test，原生 socket 採集未變，沿用既有證據。

最後唯讀快照採集 307ms（含取樣等待），逐群確認 totals 為子節點加總、ref 無重複、單一程序沒有 bin 外層；zsh 群組當次為 50 個程序（含子程序），node 為 4 個程序。這是單次觀測，非效能保證。`target/` 約 956 MiB，歸本專案作下一階段建置快取保留，階段結束再檢視；未停止既有服務。

## rw-5 Application 分組驗證

`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、25 項一般測試通過。新增 fixture 驗證 XML／binary plist、同 ID 不同名稱／版本、同名不同 ID 不合併、單一路徑不增層、缺失／無效／超限 metadata 降級、原 ref、跨路徑 PPID、未知記憶體與 totals／uptime。原生採集未修改，沿用既有 live test 證據。

真機唯讀快照確認 Chrome 合併為 `group:e442fa24391a5877`，34 個程序、約 3.1 GiB RSS。兩個路徑保留原 ref，數量與已知 RSS 加總一致；版本分別為磁碟 bundle `156.0.8078.74` 與 `156.0.8078.48`。單次採集 350ms（含取樣等待），後續 Markdown 查詢 300ms；僅為當次觀測，非效能保證。未啟動、停止或修改 Chrome。

## rw-4 Repository 分組驗證

`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、23 項一般測試與受影響 process live test 通過。新增測試涵蓋三個 checkout 合併、相對／絕對 metadata 路徑、同名獨立 clone、main checkout 未出現在快照、單 checkout 不增層、原 ref 可查詢、部分未知／uptime 彙總，以及缺損 metadata 降級。

真機確認 `bandit-forge` 合併為 `group:df9bf3f07e62929d`，保留原 main／worktree ref。驗證期間其中一個 worktree 已沒有被採集到的程序，當時快照有主 checkout 與另一個 worktree；兩者 totals 與專案總量一致。三 checkout 情況由固定 fixture 驗證。單次採集約 278ms，含取樣等待，非效能保證。未修改使用者的 repository 或 worktree；自有測試暫存資料與 sleep 已回收。

## rw-3 摘要與詳情驗證

`cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、21 項一般測試與受影響的 process live test 通過。新增檢查覆蓋摘要不帶逐程序 metadata、零用量／部分可讀程序保留、Unreadable 合併計數與無展開標記、明確 PID 診斷，以及 JSON／總計不變。原生 socket 採集未修改，沿用既有真機證據。

真機 `--md --depth 3` 將 188 個無可讀資料的程序合併成一列；一般程序列沒有 PPID／UID、路徑、socket 詳情或逐項權限錯誤。既有 live fixture 的 sleep 與暫存目錄已回收。此輪沒有實作或執行管理員提權。

## rw-2 真機證據

一般使用者、工具沙箱外、未使用 sudo，單次最終採集：780 程序，Applications 233、System 178、Development 107、Unclassified 262；其中 189 個程序身分無法確認。591 個程序完成 socket 採集、0 個部分結果，共觀測 182 個 TCP／UDP socket（此為採集記錄數，不是跨程序去重後的系統連線總數）。17 個程序使用明示的祖先推論。

採集 275ms（含約 250ms 等待），完整執行約 290ms，0.03s user／0.01s sys，最大 RSS 10,010,624 bytes（約 9.5 MiB）；完整 JSON 約 1.18 MB。加入網路資訊後仍屬單次量測，非效能保證。

此輪修改前的工具沙箱內基線為 786 程序、Unclassified 257、身分未知 189、採集 281ms；中途新版採集 792 程序、Unclassified 266、身分未知 193。系統程序持續變動，不能當成同一資料集的分類正確率比較；移除名稱猜測也可能增加未知。中途採集的身分失敗中，192 個為 `Operation not permitted (os error 1)`，1 個為 `No such process (os error 3)`，現在會保留區別。

工具沙箱限制 `/usr/bin/time -l` 的 `sysctl kern.clockrate`；上述最終成本使用沙箱外量測。Markdown 另以自有 loopback TCP／UDP socket 驗證，顯示對應監聽埠與端點，退出即關閉。

## rw-1 歷史基線

以下為 commit `613e0a1` 的舊版規則與測量，不代表目前分類。

最終原型以一般使用者權限執行（工具沙箱外，沒有 sudo）：

| 指標 | 結果 |
| --- | --- |
| 程序數 | 868 |
| cwd 可讀 | 634 |
| executable 可讀 | 629 |
| CPU／RSS 可讀 | 各 634 |
| 分類 | Applications 188、System 206、Development 144、Container hosts 27、Unclassified 303 |
| 採集時間 | 268ms，包含約 250ms 等待；完整執行約 296ms |
| `/usr/bin/time -l` | 0.03s user、0.00s sys；最大 RSS 7,733,248 bytes（約 7.4 MiB） |

這是單次最終 smoke 的結果，並非效能保證。較早版本三次有界取樣得到約 295–298ms，僅作開發成本參考；數值隨系統負載與程序數改變。使用 `--depth all` 時輸出量約 1 MiB，日常應使用預設深度與分支查詢。

在工具沙箱內仍可取得主要程序資訊，但 swap query 被限制，呈現 `?`／`null` 並附警告；解除沙箱後可取得 swap。兩種環境都仍有其他使用者／受保護程序不可讀，不以零補值。

容器唯讀盤點：

- Docker client／server 29.9.2，context `desktop-linux`，`docker ps` 可用。
- Apple container CLI 1.1.0，`container list --format json` 回報 `XPC connection error: Connection invalid`，沒有自動啟動服務。
- PATH 未發現 Podman。未驗證其 adapter，也不宣稱支援完成。

## 驗證與保留資源

- `cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`。
- `cargo test`：19 項一般測試通過；2 項 live test 預設 ignored。新增規則先重現名稱誤判與 clone bundle 漏分，再完成修正。
- `cargo test --test live -- --ignored --test-threads=1`：2 項真機測試通過，驗證 PID／PPID、cwd、CPU／RSS、專案歸類、group 查詢、預設非互動 Markdown、無 socket，以及 TCP listener／Established、UDP、IPv4／IPv6、重複 FD 去重。IPv6 dual-stack 解碼調整後針對 socket live test 重驗通過。
- 不存在 PID：exit 1、stdout 空白、stderr 說明 target not found。
- cargo-audit 未安裝，未執行 advisory audit。
- Live fixture 的 sleep 已 kill／wait；臨時目錄由 TempDir 清除。Loopback sockets 隨 handle／測試程序退出釋放。沒有啟動容器或常駐採集程序。
- `target/` 約 655 MiB，為 reswho 後續開發保留的共用 build cache；下一階段邊界檢查大小，不與其他工作共用或任意清理其他 cache。

Docker adapter 已接入；下一階段可基於已驗證的資料接上 Ratatui，再做持續觀測與 Review。
