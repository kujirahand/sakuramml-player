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
            freq: 440.0, phase: 0.0, vel: 0.0,
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
    fn tick(&mut self, sr: f32) -> f32 {
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
        let (wave, scale) = if self.is_drum {
            // xorshift32 ホワイトノイズ
            let noise = self.rng.next_f32_signed();

            // トーン成分 (方形波) ― バスドラムやタムのピッチ感
            self.phase += self.freq / sr;
            if self.phase >= 1.0 { self.phase -= 1.0; }
            let tone = if self.phase < 0.5 { 1.0f32 } else { -1.0f32 };

            // ノイズとトーンを mix
            let w = noise * self.noise_mix + tone * (1.0 - self.noise_mix);
            (w, self.drum_level * 0.09)
        } else {
            // 方形波
            self.phase += self.freq / sr;
            if self.phase >= 1.0 { self.phase -= 1.0; }
            let w = if self.phase < 0.5 { 1.0f32 } else { -1.0f32 };
            (w, 0.07)
        };

        wave * self.vel * self.env_val * scale
    }
}

// ─────────────────────────────────────────────────────────
// シンセサイザー (複数ボイス管理)
// ─────────────────────────────────────────────────────────

pub struct PsgSynth {
    voices: Vec<Voice>,
    sr:     f32,
}

impl PsgSynth {
    pub fn new(sr: f32) -> Self {
        PsgSynth { voices: vec![Voice::new(); MAX_VOICES], sr }
    }

    pub fn reset(&mut self) {
        for v in &mut self.voices { v.reset(); }
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

    /// buf をモノラル PCM (f32) で埋める (128 サンプル単位を想定)
    pub fn process_block(&mut self, buf: &mut [f32]) {
        let sr = self.sr;
        for sample in buf.iter_mut() {
            let mut mix = 0.0f32;
            for v in &mut self.voices { mix += v.tick(sr); }
            *sample = mix.clamp(-1.0, 1.0);
        }
    }
}
