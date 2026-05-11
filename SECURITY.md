# Security Policy

## Reporting a Vulnerability

如果你發現 VocalSync Studio 有安全漏洞（例如：本地檔案路徑注入、遠端程式碼執行、透過惡意下載 URL 觸發的問題等），**請不要直接開 public issue**。

請透過以下任一方式回報：

1. **GitHub Security Advisory**（推薦）：前往
   [Security → Report a vulnerability](https://github.com/himawaril2dev/vocalsync-studio/security/advisories/new)
2. **電子信箱**：`himawaril2dev@gmail.com`（主旨註明「VocalSync Security」）

### 回報時請附上

- 影響的版本（例如 `v0.2.6`）
- 影響面（可執行程式碼？可讀取檔案？DoS？）
- 重現步驟或 proof-of-concept
- 建議的修補方向（如果有的話）

### 回應時程

- **72 小時內**會回覆確認收到
- **7 天內**會評估嚴重性並排出修補時程
- 修補後會在 Release Notes 致謝（除非你希望匿名）

## Supported Versions

僅最新的 minor version 會收到安全更新。

| 版本 | 支援狀態 |
|---|---|
| 0.2.x | ✅ 支援中 |
| 0.1.x | ❌ 不再維護（請升級到 0.2.x）|

## 分發來源

**唯一官方分發管道**是
[GitHub Releases](https://github.com/himawaril2dev/vocalsync-studio/releases)。

從任何第三方網站下載的 zip 可能遭到竄改——請用 SHA256 比對 release 頁顯示的 digest。

```powershell
certutil -hashfile "VocalSync.Studio.Portable.x.y.z.zip" SHA256
```
