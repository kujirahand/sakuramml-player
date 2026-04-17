pub mod midi_parser;
pub mod player;
pub mod synth;
pub mod synth_psg;
pub mod utils;
pub mod soundfont;
use wasm_bindgen::prelude::*;

/// ブラウザ向け MIDI プレイヤー (Rust/Wasm)
///
/// ## 典型的な使い方 (チャンク再生)
/// ```js
/// const p = new MidiPlayer(44100);
/// const json = p.load(uint8array);      // 解析 & カーソル初期化
/// // seek_toで任意位置に移動 (0 = 先頭)
/// p.seek_to(0.0);
/// while (!p.is_render_done()) {
///   const pcm = p.render_next(441000); // 10 秒分
///   // → AudioBufferSourceNode にスケジュール
/// }
/// ```
#[wasm_bindgen]
pub struct MidiPlayer {
    inner: player::Player,
}

#[wasm_bindgen]
impl MidiPlayer {
    /// sample_rate: AudioContext.sampleRate (通常 44100 または 48000)
    #[wasm_bindgen(constructor)]
    pub fn new(sample_rate: f32) -> MidiPlayer {
        MidiPlayer {
            inner: player::Player::new(sample_rate),
        }
    }

    // ─────────────────────────────────────────
    // ロード & メタ情報
    // ─────────────────────────────────────────

    /// MIDI バイト列を読み込み、ノートイベントを JSON 文字列で返す。
    /// 内部でイベントリストを構築しカーsorを先頭に設定する。
    pub fn load(&mut self, data: &[u8]) -> Result<String, JsValue> {
        self.inner
            .load(data)
            .map_err(|e| JsValue::from_str(&e))
    }

    /// 曲の総再生時間 (秒)
    pub fn get_duration(&self) -> f64 {
        self.inner.get_duration()
    }

    /// 全ノートイベントを JSON 配列文字列で返す (ピアノロール描画用)
    pub fn get_note_events_json(&self) -> String {
        self.inner.get_note_events_json()
    }

    /// 総サンプル数を返す
    pub fn get_total_samples(&self) -> u32 {
        self.inner.get_total_samples() as u32
    }

    // ─────────────────────────────────────────
    // ストリーミングレンダリング API
    // ─────────────────────────────────────────

    /// 再生カーソルを `time_sec` 秒へシーク。
    /// その位置で鳴っているノートをシンセに即ロードする。
    pub fn seek_to(&mut self, time_sec: f64) {
        self.inner.seek_to(time_sec);
    }

    /// カーソル位置から `frames` サンプル分を PCM に変換して返す。
    /// 末尾に達した場合は長さ 0 の Float32Array を返す。
    ///
    /// JS 側は返り値を `AudioBuffer.copyToChannel()` に渡してスケジュールする。
    pub fn render_next(&mut self, frames: u32) -> Vec<f32> {
        self.inner.render_next(frames as usize)
    }

    /// 全サンプルを出力し終えたか (render_next が空を返す状態)
    pub fn is_render_done(&self) -> bool {
        self.inner.is_render_done()
    }

    /// 現在のレンダーカーソル位置 (サンプル数)
    pub fn get_render_pos(&self) -> u32 {
        self.inner.pos_sample as u32
    }
}

/// MMLテキストをMIDIバイト列(Uint8Array)とログ文字列にコンパイルする
#[wasm_bindgen]
pub struct CompileResult {
    bin: Vec<u8>,
    log: String,
}

#[wasm_bindgen]
impl CompileResult {
    #[wasm_bindgen(getter)]
    pub fn bin(&self) -> Vec<u8> {
        self.bin.clone()
    }

    #[wasm_bindgen(getter)]
    pub fn log(&self) -> String {
        self.log.clone()
    }
}

#[wasm_bindgen]
pub fn compile_mml(source: &str) -> CompileResult {
    let res = sakuramml::compile(source, sakuramml::SAKURA_DEBUG_NONE);
    CompileResult {
        bin: res.bin,
        log: res.log,
    }
}

/// Shift_JISの可能性があるバイト列をUTF-8文字列に変換する
#[wasm_bindgen]
pub fn encoding_to_utf8(data: &[u8]) -> String {
    // UTF-8として正常に解釈できるか確認
    if let Ok(s) = std::str::from_utf8(data) {
        return s.to_string();
    }
    // UTF-8でなければShift_JISとみなしてデコード
    let (cow, _encoding_used, _had_errors) = encoding_rs::SHIFT_JIS.decode(data);
    cow.into_owned()
}
