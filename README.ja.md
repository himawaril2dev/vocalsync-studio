# VocalSync Studio

[繁體中文](README.md) | [English](README.en.md) | **日本語**

デスクトップのボーカル練習室——歌を練習したい、自分の声をちゃんと聴きたいあなたのために。練習のたびに、上達を耳で実感できる。

VocalSync Studio は、伴奏再生・リアルタイム録音・AI ピッチ検出・歌詞同期表示を組み合わせたデスクトップアプリケーションです。歌唱練習者が自分のパフォーマンスを視覚的に確認できるよう支援します。

📖 **ユーザーガイド**：[日本語](docs/USER_GUIDE.ja.md)｜[繁體中文](docs/USER_GUIDE.md)｜[English](docs/USER_GUIDE.en.md)（portable zip にはオフライン HTML 版も同梱）

> 📢 **開示事項**
> 作者にはプログラミングの経験がありません。本プロジェクトは AI（Claude / Codex）との協働により開発され、アーキテクチャ・コード・UI まですべて AI によって生成されました。すべての機能は実テストとマルチモデルのコードレビュー（Claude による実装 + Codex による独立監査）を経ています。ご自身の用途に応じてリスクをご判断の上ご利用ください。

## 主な機能

- **YouTube ダウンロード** — URL を入力するだけで伴奏をダウンロード（yt-dlp + FFmpeg を自動インストール）
- **リアルタイム録音** — 伴奏を聴きながら録音でき、レイテンシーキャリブレーションにも対応
- **AI ピッチ検出** — CREPE ニューラルネットワークモデルで歌声のピッチを解析（オフライン動作、ネットワーク不要）
- **ピッチカーブ比較** — 自分の歌唱と目標メロディを並べて表示
- **ガイドボーカル監聴** — 読み込んだボーカルトラックを低音量の参考音として使い、エクスポートには混ぜません
- **歌詞同期** — LRC / SRT / VTT 形式に対応し、バイリンガル字幕の自動判定も可能
- **A-B ループ** — 特定区間を繰り返し練習
- **速度変更** — WSOLA タイムストレッチでピッチを変えずに再生速度を変更

## 技術スタック

| レイヤー | 技術 |
|------|------|
| フロントエンド | Svelte 5 + TypeScript + Vite |
| バックエンド | Rust + Tauri v2 |
| オーディオ | cpal（録音／再生）+ symphonia（デコード）+ biquad（フィルタリング）|
| ピッチ検出 | CREPE tiny（ONNX Runtime）+ PYIN（従来アルゴリズム）|
| 信号処理 | rustfft（FFT）+ WSOLA（タイムストレッチ）|
| ダウンロード | yt-dlp CLI ラッパー + SHA-256 サプライチェーン検証 |

## インストール

### Release からダウンロード（推奨）

1. [Releases](https://github.com/himawaril2dev/vocalsync-studio/releases) ページから最新の `VocalSync.Studio.Portable.x.y.z.zip` をダウンロードします
2. 任意の場所（デスクトップや `D:\Tools\` など）に展開します
3. フォルダを開き、**`vocalsync-studio.exe`** をダブルクリックすると起動します

> ⚠️ **フォルダ構成を変更しないでください**
> `DirectML.dll`、`yt-dlp.exe`、`models/` は `vocalsync-studio.exe` が起動時に読み込む依存ファイルです。**個別に移動させないでください。** アプリを別の場所に移したい場合は、フォルダごとまとめて移動してください。

| ファイル | 説明 |
|---|---|
| **`vocalsync-studio.exe`** | 本体 ← これを起動 |
| `DirectML.dll` | ONNX Runtime の DirectX ML アクセラレーション DLL（CREPE ピッチ検出で必要）|
| `yt-dlp.exe` | YouTube 伴奏ダウンロード CLI |
| `models/crepe-tiny.onnx` | CREPE ピッチ検出モデル |

### 動作確認済み環境

| 項目 | バージョン |
|---|---|
| OS | Windows 11 Pro 23H2 / 24H2 |
| アーキテクチャ | x86_64 |
| WebView2 | 120+（Windows 10/11 にプリインストール）|

macOS / Linux 向けの Tauri ビルドは理論上可能（ソースはクロスプラットフォーム）ですが **未検証** で、portable release も提供していません。これらのプラットフォームでビルドしたい場合は「ソースからビルド」手順に従い、結果は Issues で報告していただけると助かります。

> **初回起動時の Windows SmartScreen 警告について**
> 現時点ではコード署名（code-signing）を行っていないため、Windows SmartScreen に「WindowsによってPCが保護されました」という警告が表示される場合があります。
> 警告ウィンドウ左上の **「詳細情報」** をクリックし、下に表示される **「実行」** ボタンを押せば起動できます。
> 同じ exe に対してこの警告は二度目以降表示されません。
> 入手元に不安がある場合は、Release ページから zip をダウンロードした後、`certutil -hashfile "ファイル名.zip" SHA256` で GitHub に掲載されているダイジェストと照合してください。

### ソースからビルド

**必要な環境：**
- [Node.js](https://nodejs.org/) >= 18
- [Rust](https://rustup.rs/) >= 1.77
- [Tauri CLI](https://v2.tauri.app/start/prerequisites/)

```bash
# 1. フロントエンド依存関係をインストール
npm install

# 2. 開発モード
npm run tauri dev

# 3. Release ビルド
npm run tauri build

# 4.（任意）オフライン USER_GUIDE HTML を生成して portable zip に同梱
npm run build:docs          # dist-docs/*.html を出力
npm run pack:portable-docs  # portable フォルダに HTML を入れて zip を作り直し
```

ビルド成果物は `src-tauri/target/release/bundle/` に出力されます。

## プロジェクト構成

```
vocalsync-studio-tauri/
├── src/                    # Svelte フロントエンド
│   ├── components/         # UI コンポーネント（ピッチカーブ、歌詞パネル、キャリブレーター ...）
│   ├── stores/             # ステート管理（再生、録音、歌詞、設定 ...）
│   └── tabs/               # ページ（準備、録音、ピッチ、アバウト）
├── src-tauri/
│   └── src/
│       ├── commands/       # Tauri IPC コマンド
│       ├── core/           # コアエンジン
│       │   ├── audio_engine.rs      # 録音／再生エンジン
│       │   ├── crepe_engine.rs      # CREPE AI ピッチ検出
│       │   ├── pyin_engine.rs       # PYIN 従来型ピッチ検出
│       │   ├── lyrics_parser.rs     # LRC/SRT/VTT パーサー
│       │   ├── wsola.rs             # タイムストレッチ
│       │   ├── ytdlp_engine.rs      # YouTube ダウンロード
│       │   └── ...
│       └── lib.rs          # Tauri エントリポイント
└── package.json
```

## ショートカットキー

| キー | 機能 |
|------|------|
| `Space` | 再生 / 一時停止 |
| `R` | 録音開始 |
| `Esc` | 停止 |
| `A` | ループ A 点を設定 |
| `B` | ループ B 点を設定 |
| `+` | 半音上げる |
| `-` | 半音下げる |

## 問題報告

使用中に困ったこと、バグ報告、機能リクエストなど、いつでも歓迎しています。以下のいずれかの方法でご連絡ください。

- **GitHub Issues**：[vocalsync-studio/issues](https://github.com/himawaril2dev/vocalsync-studio/issues)
- **メール**：`himawaril2dev@gmail.com`

ご連絡の際、OS のバージョン、VocalSync Studio のバージョン、操作手順、エラーメッセージのスクリーンショットを添えていただけると、原因を特定しやすくなります。

## ライセンス

本プロジェクトは [MIT License](LICENSE) のもとでオープンソースとして公開されており、自由に使用・改変・再配布できます（無保証）。

### サードパーティコンポーネントのライセンス

| コンポーネント | ライセンス | 本プロジェクトでの用途 |
|---|---|---|
| [CREPE](https://github.com/marl/crepe) / [onnxcrepe v1.1.0](https://github.com/yqzhishen/onnxcrepe) | MIT | AI ピッチ検出モデル（NYU MARL により開発。ONNX 変換版は onnxcrepe v1.1.0 より。[BibTeX 引用](#crepe-論文引用)）|
| [ONNX Runtime](https://github.com/microsoft/onnxruntime) | MIT | CREPE モデルを実行する推論エンジン |
| [DirectML](https://github.com/microsoft/DirectML) | MIT | Windows 上の ML アクセラレーション層（`DirectML.dll`）|
| [Tauri](https://github.com/tauri-apps/tauri) | MIT / Apache-2.0 | デスクトップアプリケーションフレームワーク |
| [Svelte](https://github.com/sveltejs/svelte) | MIT | フロントエンド UI フレームワーク |
| [Symphonia](https://github.com/pdeljanov/Symphonia) / [cpal](https://github.com/RustAudio/cpal) / [rustfft](https://github.com/ejmahler/RustFFT) / [biquad](https://github.com/korken89/biquad-rs) その他 Rust crate | MIT / Apache-2.0 | オーディオデコード、録音／再生、信号処理 |

### yt-dlp（Unlicense / パブリックドメイン）

[yt-dlp](https://github.com/yt-dlp/yt-dlp) は youtube-dl のコミュニティメンテナンス fork で、**[The Unlicense](https://github.com/yt-dlp/yt-dlp/blob/master/LICENSE)**（パブリックドメイン相当、CC0-like）の下で公開されています。

- **使用方法**：本プロジェクトは `yt-dlp.exe` を **サブプロセス / CLI として呼び出す**のみで、静的リンクもソース改変も行っていません
- **入手元**：`yt-dlp.exe` は [yt-dlp 公式 Releases](https://github.com/yt-dlp/yt-dlp/releases) からダウンロードされた未改変のバイナリです
- **ユーザーの責任**：yt-dlp を通じてダウンロードしたコンテンツが現地の法律（著作権、YouTube 利用規約など）に適合しているかは、ユーザーご自身の責任となります
- 公式ライセンス全文：<https://github.com/yt-dlp/yt-dlp/blob/master/LICENSE>

### FFmpeg（LGPL 2.1+ / GPL の可能性あり）

[FFmpeg](https://ffmpeg.org/) はデフォルトで **[LGPL-2.1-or-later](https://ffmpeg.org/legal.html)** で公開されています。libx264 や libx265 など特定のエンコーダを有効化すると **GPL-2.0-or-later** に切り替わります。

- **使用方法**：本プロジェクトは `ffmpeg` / `ffprobe` 実行ファイルを **サブプロセス / CLI として呼び出す**のみで、libavcodec / libavformat などのライブラリを静的リンクしていません。したがって本プロジェクト自体は LGPL / GPL の影響を受けません
- **入手元**：デフォルトでは [gyan.dev FFmpeg Windows builds](https://www.gyan.dev/ffmpeg/builds/) またはシステムの既存インストールから読み込まれます。ダウンロードする build のライセンスはご自身でご確認ください（essentials build は通常 LGPL、full build は GPL コンポーネントを含みます）
- **改変 / 再配布**：FFmpeg バイナリを本プロジェクトと一緒に配布する場合、FFmpeg のライセンス条項を遵守する必要があります（ライセンステキストの同梱、ソース取得方法の提示など）。本リポジトリにはライセンス衝突を避けるため ffmpeg バイナリを**同梱していません**
- 公式ライセンス全文：<https://ffmpeg.org/legal.html>

### CREPE 論文引用

本プロジェクトで使用している CREPE ピッチ検出モデルは、以下の論文を基にしています：

```bibtex
@inproceedings{kim2018crepe,
  title={{CREPE}: A Convolutional Representation for Pitch Estimation},
  author={Kim, Jong Wook and Salamon, Justin and Li, Peter and Bello, Juan Pablo},
  booktitle={2018 IEEE International Conference on Acoustics, Speech and Signal Processing (ICASSP)},
  pages={161--165},
  year={2018},
  organization={IEEE}
}
```

VocalSync のピッチ検出結果を学術研究や商用研究で使用される場合は、上記論文と [onnxcrepe](https://github.com/yqzhishen/onnxcrepe) の ONNX 変換作業も合わせて引用してください。

### 免責事項

VocalSync Studio はあくまでローカル環境での歌唱練習補助ツールです。本ツールを使ってダウンロード・処理・書き起こしされたすべてのオーディオコンテンツの著作権および合法的な使用責任は、使用者ご自身に帰属します。開発者は使用者による不適切な利用について一切責任を負いません。

## 開発を支援する

このツールが歌唱練習のお役に立ちましたら、よろしければコーヒーを一杯ごちそうしていただけると嬉しいです：

[![ko-fi](https://ko-fi.com/img/githubbutton_sm.svg)](https://ko-fi.com/himawari168)
