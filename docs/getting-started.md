# はじめに

このドキュメントは、`sakuramml-player` をローカルで起動し、ブラウザ版と CLI 版の両方を試すための手順をまとめたものです。

## 必要なもの

- Rust ツールチェーン
- `cargo`
- `wasm-pack`
- Python 3 などの簡易 HTTP サーバー
- ブラウザ

`build.sh` は `wasm-pack` に依存します。未導入なら README の案内どおり導入してください。

## 初回セットアップ

### 1. リポジトリのルートへ移動

```bash
cd /Users/kujirahand/repos/sakuramml-player
```

### 2. Wasm をビルド

```bash
./build.sh
```

実行結果として `www/pkg/` に Wasm と JS バインディングが生成されます。

### 3. ブラウザ用サーバーを起動

```bash
cd www
python3 -m http.server 8080
```

### 4. アプリを開く

ブラウザで以下を開きます。

```text
http://localhost:8080
```

## ブラウザ版の使い方

- `開く` ボタンまたはドラッグ&ドロップで `.mid` / `.midi` / `.mml` を読み込む
- MML を直接テキストエリアに入力して再生する
- `▶` で再生、`⏹` で停止
- シークバーで移動
- ズームボタンでピアノロールの時間軸を拡大縮小
- 音量スライダーで出力レベルを変更

MML を読み込んだ場合は、左側テキストエリアにソースが入り、右側にコンパイルログが表示されます。

## CLI 版の使い方

### MIDI / MML を再生

```bash
cargo run --bin sakuramml-player -- test.mid
```

```bash
cargo run --bin sakuramml-player -- test.mml
```

既定ではストリーム再生です。再生開始を早くしつつ、別スレッドで PCM を先読みして供給します。

全曲を先にレンダリングしてから再生したい場合は、次のように `--render-all` を付けます。

```bash
cargo run --bin sakuramml-player -- --render-all test.mid
```

CLI は起動時に `www/fonts/TimGM6mb.sf2` を読み込み、SoundFont が利用できる場合はそちらを優先して鳴らします。

### WAV に書き出す

```bash
cargo run --bin sakuramml-player -- test.mid output.wav
```

出力は 32-bit float / stereo WAV です。

## 開発時によく触る場所

- `src/lib.rs`
  Wasm 公開 API
- `src/player.rs`
  MIDI イベント列から PCM を作る再生制御
- `src/synth.rs`
  SoundFont / PSG の切り替えとミックス
- `www/app.js`
  ブラウザ UI の本体
- `www/index.html`
  画面レイアウト

## 既知の前提と補足

- ブラウザ側は現在 `AudioBufferSourceNode` ベースのチャンク先読み再生です。
- `www/audio-worklet-processor.js` は将来の AudioWorklet 化に向けた土台で、現状は未接続です。
- MML や MIDI のテキストイベントは Rust 側で抽出し、JS 側で歌詞や Marker として表示に振り分けます。

## 関連ドキュメント

- [CLI 再生ノイズ調査メモ](./cli-audio-noise.md)
- [CLI 再生まわりの学び](./cli-playback-lessons.md)
- [アーキテクチャ](./architecture.md)
