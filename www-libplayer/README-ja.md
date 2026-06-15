# sakuramml-libplayer

ブラウザでサクラ MML / MIDI を再生するための JavaScript ライブラリです。Rust / WebAssembly で MML コンパイル、MIDI 解析、PCM 生成を行い、JavaScript 側で Web Audio API に流し込みます。

## 使い方

```js
import { SakuraPlayer } from 'sakuramml-libplayer';

const player = new SakuraPlayer();
await player.init();
await player.loadSoundFont('/fonts/TimGM6mb.sf2');
await player.play('テンポ120 ドレミファソラシド');
```

CDN 経由で使う場合は、配信先の URL から ES module として読み込みます。

```html
<script type="module">
  import { SakuraPlayer } from 'https://cdn.jsdelivr.net/npm/sakuramml-libplayer@0.1.1/sakura-mml-player.js';

  const player = new SakuraPlayer();
  await player.init();
  await player.loadSoundFont('https://example.com/TimGM6mb.sf2');
  await player.play('テンポ120 ドレミファソラシド');
</script>
```

SoundFont はパッケージに同梱していません。利用側で URL またはバイト列を用意して、`loadSoundFont()` に渡してください。

```js
await player.loadSoundFont('https://example.com/TimGM6mb.sf2');
```

## API

- `compileMML(source)`
  MML を MIDI バイト列へ変換し、`{ midi, log }` を返します。
- `loadSoundFont(source)`
  SoundFont を読み込みます。URL、`Uint8Array`、`ArrayBuffer`、`Blob` を指定できます。
- `play(input)`
  MML 文字列または MIDI バイト列を再生します。引数なしの場合は一時停止位置から再開します。
- `pause()`
  一時停止します。
- `stop()`
  停止して先頭に戻します。
- `setPosistion(seconds)`
  再生位置を秒単位で変更します。`setPosition(seconds)` も同じ動作です。
- `setLength(seconds)`
  再生終了位置を秒単位で制限します。
- `onEvent(name, handler)`
  `compile`、`load`、`play`、`pause`、`stop`、`position`、`ended` などを購読します。

## パッケージ内容

- `sakura-mml-player.js`
  公開 JavaScript API です。
- `pkg/`
  `wasm-pack` で生成した WebAssembly バンドルです。
