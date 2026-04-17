# プロジェクト構成

このリポジトリは、Rust 側で再生エンジンを持ち、`www/` 側でブラウザ UI を持つ構成です。現状の主要ファイルを役割ごとに整理します。

## ルート

- `Cargo.toml`
  Rust クレート定義。`cdylib` と `rlib` を生成し、CLI バイナリ `sakuramml-player` も持ちます。
- `build.sh`
  `wasm-pack` を使って `www/pkg/` に Wasm 出力を作るビルドスクリプトです。
- `README.md`
  プロジェクト概要と導入手順です。
- `LICENSE`
  ライセンスです。
- `AGENTS.md`
  このリポジトリ向けの作業指示です。

## Rust ソース `src/`

### エントリポイント

- `src/lib.rs`
  Wasm から呼ぶ公開 API を定義します。`MidiPlayer`、`compile_mml`、`encoding_to_utf8` を公開しています。
- `src/main.rs`
  CLI 版の入口です。MIDI / MML の読み込み、再生、WAV 書き出しを担当します。

### 中核モジュール

- `src/midi_parser.rs`
  Standard MIDI File パーサーです。ノート、コントロール、テンポ、拍子、歌詞、Marker などを抽出します。
- `src/player.rs`
  MIDI 解析結果をサンプル単位イベントに変換し、チャンクごとに PCM を生成します。
- `src/synth.rs`
  SoundFont シンセと PSG シンセを統合するハイブリッド音源です。
- `src/synth_psg.rs`
  自作 PSG 音源です。方形波、ドラム風ノイズ、簡易リバーブなどを持ちます。
- `src/soundfont.rs`
  `rustysynth` を使った SoundFont 読み込み管理です。
- `src/utils.rs`
  乱数などの補助処理です。

### 補助 / 実験コード

- `src/bin/test_parser.rs`
  MIDI テキストイベント確認用の小さな補助バイナリです。
- `src/bin/dump_bytes.rs`
  調査途中の補助コードです。
- `src/scratch.rs`
- `src/test2.rs`

上のファイルは本流のアプリ本体ではなく、試験用または調査用として置かれています。

## フロントエンド `www/`

- `www/index.html`
  画面の骨組みです。ヘッダー、再生コントロール、MML エディタ、ピアノロールを定義します。
- `www/app.js`
  ブラウザ版アプリ本体です。Wasm 初期化、SoundFont 読み込み、ファイル入力、再生、シーク、描画、歌詞表示を担当します。
- `www/style.css`
  UI のスタイルです。
- `www/audio-worklet-processor.js`
  将来の AudioWorklet 移行用スケルトンです。
- `www/fonts/TimGM6mb.sf2`
  既定の SoundFont です。

### ビルド生成物

- `www/pkg/`
  `wasm-pack build` により生成される Wasm バンドルです。通常はビルド後に作られます。

## ドキュメント `docs/`

- `docs/getting-started.md`
  セットアップ手順です。
- `docs/project-structure.md`
  このファイルです。
- `docs/architecture.md`
  システム構成とデータフローの説明です。
- `docs/cli-audio-noise.md`
  CLI 再生ノイズ調査の記録です。
- `docs/cli-playback-lessons.md`
  CLI 再生の設計判断と学びの整理です。
- `docs/midi.md`
  MIDI のテキストイベントや文字コード処理で得られた知見です。

## サンプル / テスト用データ

曲データが、`sameples/` フォルダに入っています。

読み込みや再生の確認に使えるサンプルです。

## 現時点で把握している構造上のポイント

- 本番相当の処理は `src/` と `www/app.js` に集約されています。
- AudioWorklet はまだ本番経路に入っていません。
- ドキュメント化されていない調査用ファイルが一部残っているため、今後は `scratch` 系を整理すると見通しがさらに良くなります。
