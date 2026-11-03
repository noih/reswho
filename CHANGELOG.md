# Changelog

## 1.0.0

首次發佈準備；實際公開時間以 GitHub Release 為準。

- macOS 唯讀程序與資源歸屬，依 application、project、executable 分組。
- Ratatui TUI、Markdown、版本化 JSON 與離線 JSON Schema；排序、搜尋、範圍查詢與程序詳細資訊。
- Docker、Apple Container、Podman 探查；顯示缺失、部分資料與診斷，不啟動 runtime。
- 原生 extension discovery、安裝與隔離執行；來源優先順序、多維 tags 與 typed resources。
- Settings popup、離線設定查詢、`--config` 與 `--set KEY=VALUE`。
- Apple Silicon 預編譯 tar.gz、SHA256 與供 `noih/tap` 更新的 `reswho.rb`；MIT 授權。

已知限制：

- 第一版僅以 macOS Apple Silicon 為支援目標；deployment target 為15.0，不代表已測過所有 macOS 版本。首輪 macOS 15 Actions 驗證待執行。
- 程序資料受作業系統權限、程序退出與取樣競態影響；數據可能缺失或不完整。唯讀監測不代表永不寫檔：設定、extension 安裝與 discovery metadata 會寫入 config root。
- Apple Container stats 僅啟用已驗證的1.6.1 wire version；其他版本保留 inventory 與診斷。Podman 真實驗證僅涵蓋 nested Linux 4.9.4，macOS machine、SSH、rootless 與新版尚未驗證。
- 外部 extension 以使用者權限執行，並非安全 sandbox。使用者需信任來源。
- 不提供穩定 Rust library API。JSON 與 extension 協定以各自版本契約為準。
- 尚未提供 Developer ID 簽章或 notarization。首發前仍需真實 Actions、公開下載、Homebrew 與 crates.io 發佈驗證。
