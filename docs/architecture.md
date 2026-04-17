# アーキテクチャ

`sakuramml-player` は、Rust を音声処理の中核、JavaScript を UI と描画の中核に置く構成です。ブラウザ版では Rust を WebAssembly として読み込み、CLI 版では同じ再生エンジンをネイティブ実行します。

## 全体像

```text
MML / MIDI ファイル
    |
    |  読み込み
    v
JavaScript UI (www/app.js)
    |
    |  MML なら compile_mml()
    |  MIDI ならそのまま渡す
    v
Rust/Wasm API (src/lib.rs)
    |
    |  player.load()
    v
MIDI パーサー (src/midi_parser.rs)
    |
    |  ノート / 拍 / テキスト / CC / ProgramChange を抽出
    v
プレイヤー (src/player.rs)
    |
    |  サンプル単位イベント列へ変換
    |  seek / render_next を提供
    v
シンセ (src/synth.rs)
    |\
    | \- SoundFont 再生 (rustysynth)
    |
    \--- PSG 再生 (src/synth_psg.rs)
    |
    v
PCM (f32 stereo interleaved)
    |
    v
Web Audio API / rodio
```

## 責務分担

### Rust 側

- MIDI / MML 由来データの解析
- ノート状態とコントロール状態の保持
- テンポマップと拍情報の計算
- テキストイベント抽出
- PCM 生成
- SoundFont と PSG の切り替え

### JavaScript 側

- ファイル選択とドラッグ&ドロップ
- MML エディタとコンパイルログ表示
- 再生、停止、シーク、音量、ズーム
- ピアノロール描画
- 歌詞や Marker の表示制御
- AudioContext へのスケジューリング

## 再生フロー

### 1. 読み込み

- `.mml` は `compile_mml()` で MIDI バイト列へ変換
- `.mid` / `.midi` はそのまま `player.load()` に渡す
- `player.load()` は `midi_parser::parse()` を呼び、解析結果から JSON を返す

この JSON には少なくとも以下が入ります。

- ノート列
- 小節 / 拍の情報
- テキストイベント列

これを JS 側が受け取ってピアノロール描画に使います。

### 2. 再生準備

- JS 側で `MidiPlayer.seek_to()` を呼び、開始位置を決める
- `Player` はその位置で鳴っているノートと有効なコントロール値を復元する
- SoundFont は事前に `load_soundfont()` で読み込まれる

### 3. チャンクレンダリング

- JS 側は `render_next(CHUNK_FRAMES)` を呼ぶ
- Rust 側は 128 フレーム単位でイベントを適用しつつ PCM を生成する
- JS 側は得られた PCM を `AudioBufferSourceNode` に積み、先読み再生する

現状は「まとめて生成したチャンクを順次スケジュールする」方式です。`audio-worklet-processor.js` は将来のリアルタイム処理用で、まだ本線では使っていません。

## ハイブリッド音源

`src/synth.rs` では、Bank Select の値で音源を切り替えます。

- CC#0 が `100` のとき
  自作 PSG 音源を使う
- それ以外
  SoundFont 音源を使う

この設計により、MML 由来の特殊パートや意図的なチップチューン風音色を PSG で鳴らしつつ、通常パートは SoundFont で鳴らせます。

## テキストイベント処理

`src/midi_parser.rs` は `0x01..=0x07` のメタイベントを抽出します。主用途は次のとおりです。

- Lyric
  再生中の歌詞表示
- Marker / Text
  ピアノロール内の注釈表示
- Track Name など
  デバッグや将来の UI 表示用

文字コードはまず UTF-8 を試し、失敗時に Shift_JIS へフォールバックする方針です。詳しくは [docs/midi.md](./midi.md) を参照してください。

## ネイティブ実行

CLI 版の `src/main.rs` は、同じ `Player` を使って以下を行います。

- `rodio` 経由の直接再生
- WAV ファイルへの書き出し

つまり再生エンジンは Rust 側に集約されており、ブラウザ版と CLI 版で共有されています。

## 今後の拡張ポイント

- AudioWorklet ベースの真のリアルタイム処理への移行
- Wasm 側に長寿命状態をより明確に集約
- メタテキスト描画の強化
- ドキュメント化されていない試験コードの整理
