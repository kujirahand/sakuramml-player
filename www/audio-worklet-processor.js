/**
 * audio-worklet-processor.js
 *
 * リアルタイムシンセ用 AudioWorkletProcessor (将来実装予定)
 *
 * 現在の実装はオフラインレンダリング (render_all) + AudioBuffer を使用しています。
 * このファイルはリアルタイム DSP への移行のためのスケルトンです。
 *
 * 移行手順:
 *   1. wasm-pack で生成した .wasm バイナリを addModule 環境で読み込む
 *   2. SharedArrayBuffer 経由で MidiPlayer の状態を共有
 *   3. process() の中で Wasm の process_block() を呼び出す
 */

class SakuraMidiProcessor extends AudioWorkletProcessor {
  constructor() {
    super();
    this._wasmExports = null;
    this._outputPtr   = 0;
    this._ready       = false;

    this.port.onmessage = async (e) => {
      const { type } = e.data;

      if (type === 'init-wasm') {
        // メインスレッドからコンパイル済み WebAssembly.Module を受け取る
        const { module, memory } = e.data;
        try {
          const instance = await WebAssembly.instantiate(module, {
            env: { memory },
          });
          this._wasmExports = instance.exports;
          this._ready       = true;
          this.port.postMessage({ type: 'ready' });
        } catch (err) {
          this.port.postMessage({ type: 'error', message: String(err) });
        }
      } else if (type === 'note-on') {
        if (this._ready && this._wasmExports?.note_on) {
          this._wasmExports.note_on(e.data.ch, e.data.note, e.data.vel);
        }
      } else if (type === 'note-off') {
        if (this._ready && this._wasmExports?.note_off) {
          this._wasmExports.note_off(e.data.ch, e.data.note);
        }
      }
    };
  }

  /**
   * AudioWorklet エンジンから 128 フレームごとに呼ばれる。
   * Wasm が利用可能な場合は process_block() を呼び出し、
   * そうでない場合は無音を出力する。
   */
  process(_inputs, outputs) {
    const output = outputs[0];
    if (!output || output.length === 0) return true;

    const left  = output[0];
    const right  = output[1] ?? output[0];

    if (!this._ready || !this._wasmExports?.process_block) {
      // 現在は無音フォールバック (PCMはメインスレッドでレンダリング済み)
      left.fill(0);
      if (right !== left) right.fill(0);
      return true;
    }

    // TODO: Wasm メモリから Float32 バッファを取得して返す
    // const mem   = new Float32Array(this._wasmExports.memory.buffer);
    // this._wasmExports.process_block(this._outputPtr, left.length);
    // left.set(mem.subarray(this._outputPtr / 4, this._outputPtr / 4 + left.length));
    // if (right !== left) right.set(left);

    return true; // Keep processor alive
  }
}

registerProcessor('sakura-midi-processor', SakuraMidiProcessor);
