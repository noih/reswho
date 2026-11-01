# reswho

## Development

- macOS、Rust＋Ratatui，第一版唯讀。產品方向見 `docs/product-brief.md`；當前原型範圍見 `docs/collection-probe.md`。
- TUI 與命令輸出使用英文；討論與規劃文件使用繁體中文。
- 驗證方式見 `TESTING.md`。一般測試不得停止既有程序、啟動容器服務或修改外部工作。

## Issue tracking

- 本專案使用 Tenqo MCP 管理需求、任務、缺陷、進度與後續工作。
- Workspace：`Side`（ID：`efcnbsjq1p27xow`）。
- Project：`ResWho`（key：`rw`，ID：`RoNYdzMR1lkYger2JsUh1`）。
- 建立 issue 前先查找既有記錄，避免重複；不使用 GitHub Issues，也不建立鏡像記錄。
- Tenqo 無法使用時保留待同步事項並回報，不自動切換 tracker。

## Git workflow

- 本專案直接在 `main` 分支工作；除非使用者另有指示，不另開工作分支。
- 僅做本地 commit，不執行 push；由使用者推送。
- Commit subject 與 body 使用英文，不加 AI-attribution trailers。
