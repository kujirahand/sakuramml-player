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
- [MML サウンドプレイヤーサンプル](docs/www-player-sample.md)
- [CLI 再生ノイズ調査メモ](docs/cli-audio-noise.md)
- [CLI 再生まわりの学び](docs/cli-playback-lessons.md)

## クイックスタート

### ブラウザ版

1. `./build.sh`
2. `cd www`
3. `python3 -m http.server 8080`
4. `http://localhost:8080` を開く

`build.sh` は `wasm-pack build --target web --out-dir www/pkg` を実行します。

### CLI 版

ストリーム再生:

```bash
cargo run --bin sakuramml-player -- test.mid
```

MML を直接ストリーム再生:

```bash
cargo run --bin sakuramml-player -- test.mml
```

全曲を先にレンダリングしてから再生:

```bash
cargo run --bin sakuramml-player -- --render-all test.mid
```

WAV 書き出し:

```bash
cargo run --bin sakuramml-player -- test.mid output.wav
```

ヘルプ表示:

```bash
cargo run --bin sakuramml-player -- --help
```

CLI 再生モードの設計判断や調査経緯は [docs/cli-audio-noise.md](docs/cli-audio-noise.md) と [docs/cli-playback-lessons.md](docs/cli-playback-lessons.md) にまとめています。

### MML サウンドプレイヤーサンプル

ピアノロール描画を使わず、MML のコンパイルと音楽再生だけを行うサンプルを `www-player-sample/` に用意しています。ブラウザ版と同じ `www/pkg/` の WebAssembly を利用します。

```bash
./build.sh
python3 -m http.server 8080
```

`http://localhost:8080/www-player-sample/` を開くと、テキストボックスと再生、一時停止、停止ボタンだけの簡単なサンプルを試せます。

## 開発の前提

- Rust 2021
- `wasm-pack`
- ブラウザの Web Audio API
- `www/fonts/TimGM6mb.sf2` の配置

## 実装の見取り図

このリポジトリは、Rust 側の再生エンジンをブラウザ版と CLI 版で共有します。ブラウザ版では `wasm-pack` で生成した Wasm を `www/app.js` から呼び出し、CLI 版では同じ `Player` をネイティブ実行して再生や WAV 書き出しを行います。

処理の大まかな流れは次の通りです。

1. `www/app.js` が MML / MIDI ファイルを読み込みます。
2. MML の場合は `compile_mml()` または `compile_mml_bytes()` で MIDI バイト列へ変換します。
3. `MidiPlayer.load()` が `src/midi_parser.rs` を使って MIDI を解析します。
4. `src/player.rs` がノート、拍、テキスト、コントロールイベントをサンプル単位のイベント列へ変換します。
5. `src/synth.rs` が SoundFont と PSG を切り替えながらステレオ PCM を生成します。
6. ブラウザ版は Web Audio API、CLI 版は `rodio` または WAV 書き出しで PCM を利用します。

主な責務は次のファイルに分かれています。

- `src/lib.rs`
  Wasm 公開 API です。`MidiPlayer`、MML コンパイル、文字コード変換、SoundFont 読み込みを公開します。
- `src/midi_parser.rs`
  Standard MIDI File を解析し、ノート、テンポ、拍子、CC、ProgramChange、PitchBend、歌詞や Marker などのメタイベントを抽出します。
- `src/player.rs`
  `seek_to()` と `render_next()` を持つチャンクレンダリングの中心です。
- `src/synth.rs`
  SoundFont と自作 PSG のハイブリッド音源です。Bank Select の CC#0 が `100` の場合は PSG、それ以外は SoundFont で鳴らします。
- `src/synth_psg.rs`
  方形波、ドラム風ノイズ、簡易リバーブなどを持つ PSG 音源です。
- `src/main.rs`
  CLI 版の入口です。MIDI / MML の再生、全曲レンダリング、WAV 書き出しを担当します。
- `www/app.js`
  ブラウザ UI、MML エディタ、ファイル読み込み、再生制御、シーク、ズーム、ピアノロール描画、歌詞表示を担当します。

## 現在の実装状況

- `src/lib.rs` で Wasm 向け API を公開
- `src/player.rs` でチャンク単位レンダリングを実装
- `src/synth.rs` で SoundFont と PSG を統合
- `www/app.js` で UI、ファイル読込、再生制御、描画を実装
- `www/audio-worklet-processor.js` は将来のリアルタイム DSP 用スケルトン

## 開発メモ

- Rust の単体テストは `src/midi_parser.rs`、`src/player.rs`、`src/main.rs` などにあり、MIDI パース、チャンクレンダリング、CLI 引数解析、WAV 書き出しを確認しています。
- `www/audio-worklet-processor.js` は現時点では本線の再生経路ではなく、将来の AudioWorklet 移行用スケルトンです。
- `src/bin/` や `scratch` 系ファイルには調査用コードが含まれており、本番相当の処理は `src/` の中核モジュールと `www/app.js` に集約されています。
