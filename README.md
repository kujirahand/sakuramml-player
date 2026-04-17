# サクラMMLプレイヤー

ブラウザで動作する MML / MIDI プレイヤーです。Rust で MIDI 解析と音声生成を行い、JavaScript で UI とピアノロール描画を担当します。現在は WebAssembly 経由のブラウザ再生と、ネイティブ実行による CLI 再生 / WAV 書き出しの両方が入っています。

## 現在の主な機能

- MML のコンパイルと MIDI 再生
- Standard MIDI File の解析
- ピアノロール表示
- 歌詞や Marker などのメタテキスト抽出
- SoundFont + 自作 PSG のハイブリッド再生
- CLI からの再生と WAV 書き出し

## ドキュメント

- [はじめに](docs/getting-started.md)
- [プロジェクト構成](docs/project-structure.md)
- [アーキテクチャ](docs/architecture.md)
- [MIDI テキストイベントの知見](docs/midi.md)

## クイックスタート

### ブラウザ版

1. `./build.sh`
2. `cd www`
3. `python3 -m http.server 8080`
4. `http://localhost:8080` を開く

`build.sh` は `wasm-pack build --target web --out-dir www/pkg` を実行します。

### CLI 版

再生:

```bash
cargo run --bin sakuramml-player -- test.mid
```

MML を直接再生:

```bash
cargo run --bin sakuramml-player -- test.mml
```

WAV 書き出し:

```bash
cargo run --bin sakuramml-player -- test.mid output.wav
```

## 開発の前提

- Rust 2021
- `wasm-pack`
- ブラウザの Web Audio API
- `www/fonts/TimGM6mb.sf2` の配置

## 現在の実装状況

- `src/lib.rs` で Wasm 向け API を公開
- `src/player.rs` でチャンク単位レンダリングを実装
- `src/synth.rs` で SoundFont と PSG を統合
- `www/app.js` で UI、ファイル読込、再生制御、描画を実装
- `www/audio-worklet-processor.js` は将来のリアルタイム DSP 用スケルトン

## 注意

- リポジトリには検証用の `test.mid` / `test.mml` / `lyric-test.mml` などが含まれています。
- `src/scratch.rs`、`src/test2.rs`、`src/bin/` 配下には試験用コードや調査用バイナリが含まれています。
