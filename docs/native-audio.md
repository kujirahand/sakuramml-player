# ネイティブ再生のチャンネル数メモ

CLI 版の `src/main.rs` では、`Player::render_next()` が返す PCM を `rodio` と WAV 出力の両方へ流しています。

今回確認できたこと:

- `Player::render_next(frames)` の返り値は `frames * 2` 要素の `f32` 配列
- 並びは stereo interleaved
- つまり `L, R, L, R, ...` の順で入っている

そのため、ネイティブ再生側で `rodio::Source::channels()` を `1` にすると、`rodio` は各 `f32` を 1ch の 1 フレームとして解釈します。結果として、本来 2ch で 4410 フレームのチャンクが、1ch で 8820 フレームあるように扱われ、体感で約 1/2 速度の再生になります。

対策:

- `Source::channels()` は `2` にする
- `current_frame_len()` はインターリーブ済みサンプル数ではなくフレーム数を見る
- WAV ヘッダも `mono` ではなく `stereo` に合わせる

補足:

- CLI で MetaText を表示するときは、生バイト列や `Debug` 表示に頼らず、`midi_parser::parse()` が返す UTF-8 化済み文字列をそのまま出す
- 文字コード判定は `UTF-8` を先に試し、失敗時だけ `Shift_JIS` にフォールバックする

## MML文字コードメモ

- `.mml` ファイルはコンパイル前に必ず UTF-8 へ正規化してから `sakuramml::compile()` に渡す
- ブラウザ版と CLI 版で別々にデコード処理を書くと片方だけ壊れやすいので、ライブラリ側の `compile_mml_bytes()` に寄せて共通化する
- `samples/bbs6-2386.mml` のような Shift_JIS 入力でも、TrackName と Copyright がそのまま読める回帰テストを維持する
