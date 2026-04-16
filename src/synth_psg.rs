//! PSG 風シンセサイザー
//!
//! - チャンネル 0〜8, 10〜15 : 方形波 + ADSR エンベロープ
//! - チャンネル 9 (MIDI channel 10 = GM パーカッション) :
//!   xorshift32 ホワイトノイズ + トーン成分 + パーカッシブ エンベロープ

use crate::utils::RandomXorShift32;

const MAX_VOICES: usize = 32;

// ─────────────────────────────────────────────────────────
// GM ドラムパラメータ
// ─────────────────────────────────────────────────────────

struct DrumParams {
    /// 減衰時間 (秒)
    decay_sec: f32,
    /// トーン成分の周波数 (Hz) ― バスドラムやタムのピッチ感に使う
    tone_freq: f32,
    /// ノイズ混合比 0.0 = 純粋なトーン, 1.0 = ノイズのみ
    noise_mix: f32,
    /// 音量補正係数
    level: f32,
}

/// GM ドラムマップ (ノート番号) に対応するパラメータを返す
fn drum_params(note: u8) -> DrumParams {
    match note {
        // ─── バスドラム ──────────────────────────────────
        35 | 36 => DrumParams { decay_sec: 0.35, tone_freq:  55.0, noise_mix: 0.30, level: 1.10 },

        // ─── スネアドラム ────────────────────────────────
        38 | 40 => DrumParams { decay_sec: 0.15, tone_freq: 200.0, noise_mix: 0.88, level: 0.90 },

        // ─── サイドスティック / ハンドクラップ ──────────
        37 | 39 => DrumParams { decay_sec: 0.08, tone_freq: 500.0, noise_mix: 1.00, level: 0.80 },

        // ─── ハイハット (クローズ) ───────────────────────
        42      => DrumParams { decay_sec: 0.04, tone_freq: 9_000.0, noise_mix: 1.00, level: 0.65 },

        // ─── ハイハット (ペダル) ─────────────────────────
        44      => DrumParams { decay_sec: 0.07, tone_freq: 8_000.0, noise_mix: 1.00, level: 0.60 },

        // ─── ハイハット (オープン) ───────────────────────
        46      => DrumParams { decay_sec: 0.42, tone_freq: 8_500.0, noise_mix: 1.00, level: 0.70 },

        // ─── タム群 ──────────────────────────────────────
        41 | 43 | 45 | 47 | 48 | 50
           => DrumParams { decay_sec: 0.20, tone_freq: 110.0, noise_mix: 0.55, level: 0.85 },

        // ─── クラッシュシンバル ──────────────────────────
        49 | 57 => DrumParams { decay_sec: 1.00, tone_freq: 6_000.0, noise_mix: 1.00, level: 0.75 },

        // ─── ライドシンバル ──────────────────────────────
        51 | 59 => DrumParams { decay_sec: 0.60, tone_freq: 7_000.0, noise_mix: 1.00, level: 0.70 },

        // ─── チャイナシンバル ────────────────────────────
        52      => DrumParams { decay_sec: 0.80, tone_freq: 5_000.0, noise_mix: 1.00, level: 0.70 },

        // ─── カウベル / ボンゴ / カスタネット等 ─────────
        56 | 60..=64
           => DrumParams { decay_sec: 0.15, tone_freq: 600.0, noise_mix: 0.60, level: 0.80 },

        // ─── その他 ─────────────────────────────────────
        _  => DrumParams { decay_sec: 0.12, tone_freq: 300.0, noise_mix: 0.80, level: 0.70 },
    }
}

// ─────────────────────────────────────────────────────────
// エンベロープ状態
// ─────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Debug)]
enum EnvState { Attack, Decay, Sustain, Release, Off }

// ─────────────────────────────────────────────────────────
// 1 ボイス
// ─────────────────────────────────────────────────────────

#[derive(Clone)]
struct Voice {
    active:  bool,
    channel: u8,
    note:    u8,
    freq:    f32,  // 通常ボイス: ノート周波数 / ドラム: トーン周波数
    phase:   f32,  // 位相 0.0〜1.0
    vel:     f32,  // ベロシティ 0.0〜1.0

    vibrato_phase: f32, // ビブラート用LFO位相

    // ── ドラム専用 ────────────────────────────
    is_drum:    bool,
    rng:        RandomXorShift32,  // xorshift32 乱数器の状態
    noise_mix:  f32,  // ノイズ混合比
    drum_level: f32,  // ドラム音量補正

    // ── エンベロープ ──────────────────────────
    env_state:   EnvState,
    env_val:     f32,
    env_samples: u32,
    attack_s:    u32,
    decay_s:     u32,
    sustain:     f32,
    release_s:   u32,
}

impl Voice {
    fn new() -> Self {
        Voice {
            active: false, channel: 0, note: 0,
            freq: 440.0, phase: 0.0, vel: 0.0, vibrato_phase: 0.0,
            is_drum: false, rng: RandomXorShift32::new(1), noise_mix: 0.0, drum_level: 1.0,
            env_state: EnvState::Off, env_val: 0.0, env_samples: 0,
            attack_s: 441, decay_s: 3528, sustain: 0.65, release_s: 11025,
        }
    }

    fn reset(&mut self) {
        self.active    = false;
        self.is_drum   = false;
        self.env_state = EnvState::Off;
        self.env_val   = 0.0;
        self.env_samples = 0;
    }

    fn note_on(&mut self, ch: u8, note: u8, vel: u8, sr: f32) {
        self.active  = true;
        self.channel = ch;
        self.note    = note;
        self.vel     = vel as f32 / 127.0;
        self.phase   = 0.0;
        self.is_drum = ch == 9; // MIDI channel 10 (0 インデックスで 9)

        if self.is_drum {
            let p = drum_params(note);
            // パーカッシブ ADSR: 高速アタック → 短ディケイ → サステインなし
            self.attack_s   = (0.002 * sr) as u32; // ~2 ms
            self.decay_s    = (p.decay_sec * sr) as u32;
            self.sustain    = 0.0; // ディケイ終了後に即消音
            self.release_s  = (0.01 * sr) as u32; // ほぼ使わない
            self.freq       = p.tone_freq;
            self.noise_mix  = p.noise_mix;
            self.drum_level = p.level;
            // xorshift32 シードをノート番号とベロシティで初期化
            let seed = (note as u32).wrapping_mul(1_000_003)
                .wrapping_add(vel as u32);
            self.rng = RandomXorShift32::new(seed);
        } else {
            self.freq      = 440.0 * 2.0_f32.powf((note as f32 - 69.0) / 12.0);
            self.attack_s  = (0.008 * sr) as u32;
            self.decay_s   = (0.07  * sr) as u32;
            self.sustain   = 0.65;
            self.release_s = (0.22  * sr) as u32;
        }

        self.env_state   = EnvState::Attack;
        self.env_val     = 0.0;
        self.env_samples = 0;
    }

    fn note_off(&mut self) {
        // ドラムは NoteOff を無視 (自然減衰に任せる)
        if self.is_drum { return; }
        if self.env_state != EnvState::Off {
            self.env_state   = EnvState::Release;
            self.env_samples = 0;
        }
    }

    /// 1 サンプルを生成して返す。非アクティブなら 0.0。
    fn tick(&mut self, sr: f32, pb_semitones: f32, mod_depth: f32) -> f32 {
        if !self.active { return 0.0; }

        // ── エンベロープ更新 ──────────────────────
        match self.env_state {
            EnvState::Attack => {
                self.env_val = if self.attack_s > 0 {
                    self.env_samples as f32 / self.attack_s as f32
                } else { 1.0 };
                self.env_samples += 1;
                if self.env_samples >= self.attack_s {
                    self.env_state   = EnvState::Decay;
                    self.env_samples = 0;
                    self.env_val     = 1.0;
                }
            }
            EnvState::Decay => {
                let t        = self.env_samples as f32 / self.decay_s.max(1) as f32;
                self.env_val = 1.0 - (1.0 - self.sustain) * t;
                self.env_samples += 1;
                if self.env_samples >= self.decay_s {
                    if self.sustain <= 0.001 {
                        // sustain=0 のドラム: そのまま消音 (Release を経由しない)
                        self.env_state = EnvState::Off;
                        self.active    = false;
                        return 0.0;
                    }
                    self.env_state   = EnvState::Sustain;
                    self.env_val     = self.sustain;
                    self.env_samples = 0;
                }
            }
            EnvState::Sustain => {
                self.env_val = self.sustain;
            }
            EnvState::Release => {
                let t        = self.env_samples as f32 / self.release_s.max(1) as f32;
                self.env_val = self.sustain * (1.0 - t).max(0.0);
                self.env_samples += 1;
                if self.env_samples >= self.release_s || self.env_val <= 0.001 {
                    self.env_state = EnvState::Off;
                    self.active    = false;
                    return 0.0;
                }
            }
            EnvState::Off => { self.active = false; return 0.0; }
        }

        // ── 波形生成 ──────────────────────────────
        let mut modulated_pb = pb_semitones;
        if !self.is_drum && mod_depth > 0.0 {
            // 5 Hz ビブラート LFO
            self.vibrato_phase += 5.0 / sr;
            if self.vibrato_phase >= 1.0 { self.vibrato_phase -= 1.0; }
            let vibrato_val = (self.vibrato_phase * std::f32::consts::PI * 2.0).sin();
            // mod_depth 1.0 = ±1.0 半音の揺れ
            modulated_pb += mod_depth * 1.0 * vibrato_val;
        }

        let (wave, scale) = if self.is_drum {
            // xorshift32 ホワイトノイズ
            let noise = self.rng.next_f32_signed();

            // トーン成分 (方形波) ― バスドラムやタムのピッチ感
            let current_freq = self.freq * 2.0_f32.powf(modulated_pb / 12.0);
            self.phase += current_freq / sr;
            if self.phase >= 1.0 { self.phase -= 1.0; }
            let tone = if self.phase < 0.5 { 1.0f32 } else { -1.0f32 };

            // ノイズとトーンを mix
            let w = noise * self.noise_mix + tone * (1.0 - self.noise_mix);
            (w, self.drum_level * 0.09)
        } else {
            // 方形波
            let current_freq = self.freq * 2.0_f32.powf(modulated_pb / 12.0);
            self.phase += current_freq / sr;
            if self.phase >= 1.0 { self.phase -= 1.0; }
            let w = if self.phase < 0.5 { 1.0f32 } else { -1.0f32 };
            (w, 0.07)
        };

        wave * self.vel * self.env_val * scale
    }
}

// ─────────────────────────────────────────────────────────
// 簡易リバーブ (Freeverb風のComb / AllPass構成)
// ─────────────────────────────────────────────────────────

struct CombFilter {
    buffer: Vec<f32>,
    idx: usize,
    feedback: f32,
    damp: f32,
    store: f32,
}

impl CombFilter {
    fn new(size: usize) -> Self {
        Self { buffer: vec![0.0; size], idx: 0, feedback: 0.84, damp: 0.2, store: 0.0 }
    }
    fn process(&mut self, input: f32) -> f32 {
        let output = self.buffer[self.idx];
        self.store = (output * (1.0 - self.damp)) + (self.store * self.damp);
        self.buffer[self.idx] = input + (self.store * self.feedback);
        self.idx = (self.idx + 1) % self.buffer.len();
        output
    }
}

struct AllPassFilter {
    buffer: Vec<f32>,
    idx: usize,
}

impl AllPassFilter {
    fn new(size: usize) -> Self {
        Self { buffer: vec![0.0; size], idx: 0 }
    }
    fn process(&mut self, input: f32) -> f32 {
        let buf_val = self.buffer[self.idx];
        let output = -input + buf_val;
        self.buffer[self.idx] = input + (buf_val * 0.5); // feedback=0.5
        self.idx = (self.idx + 1) % self.buffer.len();
        output
    }
}

struct SimpleReverb {
    combs: Vec<CombFilter>,
    allpasses: Vec<AllPassFilter>,
}

impl SimpleReverb {
    fn new(sr: f32) -> Self {
        let mut combs = Vec::new();
        let scale = sr / 44100.0;
        for &size in &[1116, 1188, 1277, 1356, 1422, 1491] {
            combs.push(CombFilter::new((size as f32 * scale).max(1.0) as usize));
        }
        let mut allpasses = Vec::new();
        for &size in &[225, 341, 441] {
            allpasses.push(AllPassFilter::new((size as f32 * scale).max(1.0) as usize));
        }
        Self { combs, allpasses }
    }
    fn process(&mut self, input: f32) -> f32 {
        let mut out = 0.0;
        for c in &mut self.combs {
            out += c.process(input);
        }
        for a in &mut self.allpasses {
            out = a.process(out);
        }
        out * 0.25
    }
}

// ─────────────────────────────────────────────────────────
// 簡易ステレオコーラス
// ─────────────────────────────────────────────────────────

struct FractionalDelay {
    buffer: Vec<f32>,
    write_pos: usize,
}

impl FractionalDelay {
    fn new(size: usize) -> Self {
        Self { buffer: vec![0.0; size], write_pos: 0 }
    }
    fn write(&mut self, val: f32) {
        self.buffer[self.write_pos] = val;
        self.write_pos = (self.write_pos + 1) % self.buffer.len();
    }
    fn read(&self, delay: f32) -> f32 {
        let len = self.buffer.len() as f32;
        let mut read_pos = self.write_pos as f32 - delay;
        while read_pos < 0.0 { read_pos += len; }
        
        let i0 = read_pos as usize % self.buffer.len();
        let i1 = (i0 + 1) % self.buffer.len();
        let frac = read_pos - (read_pos as usize) as f32;
        
        self.buffer[i0] * (1.0 - frac) + self.buffer[i1] * frac
    }
}

struct SimpleChorus {
    delay_l: FractionalDelay,
    delay_r: FractionalDelay,
    lfo_phase: f32,
    sr: f32,
}

impl SimpleChorus {
    fn new(sr: f32) -> Self {
        let max_delay = (sr * 0.05) as usize; // max 50ms buffer
        Self {
            delay_l: FractionalDelay::new(max_delay),
            delay_r: FractionalDelay::new(max_delay),
            lfo_phase: 0.0,
            sr,
        }
    }
    
    // returns (out_l, out_r)
    fn process(&mut self, input: f32) -> (f32, f32) {
        let base_delay = self.sr * 0.015; // 15ms base delay
        let mod_depth = self.sr * 0.003;  // 3ms depth
        let lfo_rate = 0.8;               // 0.8 Hz
        
        self.lfo_phase += lfo_rate / self.sr;
        if self.lfo_phase >= 1.0 { self.lfo_phase -= 1.0; }
        
        let lfo_val = (self.lfo_phase * std::f32::consts::PI * 2.0).sin();
        
        // Anti-phase for stereo width
        let d_l = base_delay + mod_depth * lfo_val;
        let d_r = base_delay - mod_depth * lfo_val;
        
        let out_l = self.delay_l.read(d_l);
        let out_r = self.delay_r.read(d_r);
        
        self.delay_l.write(input);
        self.delay_r.write(input);
        
        (out_l, out_r)
    }
}

// ─────────────────────────────────────────────────────────
// シンセサイザー (複数ボイス管理)
// ─────────────────────────────────────────────────────────

pub struct PsgSynth {
    voices: Vec<Voice>,
    sr:     f32,
    reverb: SimpleReverb,
    reverb_send: [f32; 16],
    chorus: SimpleChorus,
    chorus_send: [f32; 16],
    pitch_bend: [f32; 16], // semitones (-2.0 to +2.0)
    pan: [f32; 16],        // 0.0 = left, 0.5 = center, 1.0 = right
    modulation: [f32; 16], // 0.0 to 1.0
}

impl PsgSynth {
    pub fn new(sr: f32) -> Self {
        PsgSynth {
            voices: vec![Voice::new(); MAX_VOICES],
            sr,
            reverb: SimpleReverb::new(sr),
            reverb_send: [0.0; 16],
            chorus: SimpleChorus::new(sr),
            chorus_send: [0.0; 16],
            pitch_bend: [0.0; 16],
            pan: [0.5; 16],
            modulation: [0.0; 16],
        }
    }

    pub fn reset(&mut self) {
        for v in &mut self.voices { v.reset(); }
        self.reverb = SimpleReverb::new(self.sr);
        self.chorus = SimpleChorus::new(self.sr);
        self.pitch_bend = [0.0; 16];
        self.pan = [0.5; 16];
        self.reverb_send = [0.0; 16];
        self.chorus_send = [0.0; 16];
        self.modulation = [0.0; 16];
    }

    pub fn note_on(&mut self, ch: u8, note: u8, vel: u8) {
        let sr = self.sr;
        if let Some(v) = self.voices.iter_mut().find(|v| !v.active) {
            v.note_on(ch, note, vel, sr);
            return;
        }
        // ボイスが足りない → 最小 env_val のボイスを奪う
        if let Some(v) = self.voices.iter_mut()
            .min_by(|a, b| a.env_val.partial_cmp(&b.env_val).unwrap())
        {
            v.note_on(ch, note, vel, sr);
        }
    }

    pub fn note_off(&mut self, ch: u8, note: u8) {
        for v in &mut self.voices {
            if v.active && v.channel == ch && v.note == note {
                v.note_off();
            }
        }
    }

    pub fn set_reverb_send(&mut self, ch: u8, val: u8) {
        let ch_idx = (ch & 0x0F) as usize;
        self.reverb_send[ch_idx] = (val as f32) / 127.0;
    }

    pub fn set_chorus_send(&mut self, ch: u8, val: u8) {
        let ch_idx = (ch & 0x0F) as usize;
        self.chorus_send[ch_idx] = (val as f32) / 127.0;
    }

    pub fn set_modulation(&mut self, ch: u8, val: u8) {
        let ch_idx = (ch & 0x0F) as usize;
        self.modulation[ch_idx] = (val as f32) / 127.0;
    }

    pub fn set_pitch_bend(&mut self, ch: u8, val: u16) {
        let ch_idx = (ch & 0x0F) as usize;
        // val ranges 0..16383, center is 8192
        let pb_normalized = (val as f32 - 8192.0) / 8192.0;
        self.pitch_bend[ch_idx] = pb_normalized * 2.0; // +/- 2 semitones
    }

    pub fn set_pan(&mut self, ch: u8, val: u8) {
        let ch_idx = (ch & 0x0F) as usize;
        self.pan[ch_idx] = (val as f32) / 127.0;
    }

    /// left_buf と right_buf をステレオ PCM (f32) で埋める (128 サンプル単位を想定)
    pub fn process_block(&mut self, left_buf: &mut [f32], right_buf: &mut [f32]) {
        let sr = self.sr;
        for i in 0..left_buf.len() {
            let mut mix_l = 0.0f32;
            let mut mix_r = 0.0f32;
            let mut rev_in = 0.0f32;
            let mut chor_in = 0.0f32;
            for v in &mut self.voices {
                let ch_idx = (v.channel & 0x0F) as usize;
                let pb = self.pitch_bend[ch_idx];
                let mod_depth = self.modulation[ch_idx];
                let smp = v.tick(sr, pb, mod_depth);
                if smp != 0.0 {
                    let pan = self.pan[ch_idx];
                    let angle = pan * std::f32::consts::PI / 2.0;
                    let l_gain = angle.cos();
                    let r_gain = angle.sin();
                    
                    mix_l += smp * l_gain;
                    mix_r += smp * r_gain;
                    
                    if v.active {
                        let r_send = self.reverb_send[ch_idx];
                        rev_in += smp * r_send;
                        let c_send = self.chorus_send[ch_idx];
                        chor_in += smp * c_send;
                    }
                }
            }
            let rev_out = self.reverb.process(rev_in);
            let (chor_l, chor_r) = self.chorus.process(chor_in);
            
            // コーラスは通常ステレオ空間に広げるので、左右の出力を乗せる
            left_buf[i] = (mix_l + rev_out + chor_l).clamp(-1.0, 1.0);
            right_buf[i] = (mix_r + rev_out + chor_r).clamp(-1.0, 1.0);
        }
    }
}
