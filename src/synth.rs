//! ハイブリッド・シンセサイザー (rustysynth + PSG)
//!
//! BankSelect (CC#0) の値が 100 の場合は、自作 PSG 音源(synth_psg.rs)を利用し、
//! それ以外の場合は rustysynth (SoundFont) を利用して高品質なPCMを生成します。

use crate::soundfont::get_soundfont;
use crate::synth_psg::PsgSynth;
use rustysynth::{Synthesizer, SynthesizerSettings};

const ROUTE_NONE: u8 = 0;
const ROUTE_SF2: u8 = 1;
const ROUTE_PSG: u8 = 2;

// ─────────────────────────────────────────────────────────
// シンセサイザー (ハイブリッド)
// ─────────────────────────────────────────────────────────

pub struct Synth {
    sf2_synth: Option<Synthesizer>,
    psg_synth: PsgSynth,
    
    left_buf: Vec<f32>,
    right_buf: Vec<f32>,
    psg_buf: Vec<f32>,
    
    current_sf2_bank: [u8; 16],
    current_sf2_program: [u8; 16],
    
    // NoteOff 時に「鳴らしたシンセ」へルーティングするため、各チャンネル/ノートのルーティング先を保存
    note_routing: [[u8; 128]; 16],
}

impl Synth {
    pub fn new(sr: f32) -> Self {
        // rustysynth の設定
        let settings = SynthesizerSettings::new(sr as i32);
        
        let sf2_synth = match get_soundfont() {
            Some(sf) => {
                Some(Synthesizer::new(&sf, &settings).unwrap_or_else(|_| panic!("Failed to create Synthesizer")))
            },
            None => None,
        };

        Synth { 
            sf2_synth,
            psg_synth: PsgSynth::new(sr),
            left_buf: Vec::new(),
            right_buf: Vec::new(),
            psg_buf: Vec::new(),
            current_sf2_bank: [0; 16],
            current_sf2_program: [0; 16],
            note_routing: [[ROUTE_NONE; 128]; 16],
        }
    }

    pub fn reset(&mut self) {
        if let Some(s) = &mut self.sf2_synth {
            s.reset();
        }
        self.psg_synth.reset();
        
        self.current_sf2_bank = [0; 16];
        self.current_sf2_program = [0; 16];
        self.note_routing = [[ROUTE_NONE; 128]; 16];
    }

    pub fn note_on(&mut self, ch: u8, note: u8, vel: u8, bank: u8, program: u8) {
        let ch_idx = (ch & 0x0F) as usize;
        let note_idx = (note & 0x7F) as usize;

        // Bank 100 なら PSG を使用
        if bank == 100 {
            self.psg_synth.note_on(ch, note, vel);
            self.note_routing[ch_idx][note_idx] = ROUTE_PSG;
        } else {
            if let Some(s) = &mut self.sf2_synth {
                // CC#0 / ProgramChange を必要に応じて送信
                if self.current_sf2_bank[ch_idx] != bank {
                    s.process_midi_message(ch as i32, 0xB0, 0, bank as i32);
                    self.current_sf2_bank[ch_idx] = bank;
                }
                if self.current_sf2_program[ch_idx] != program {
                    s.process_midi_message(ch as i32, 0xC0, program as i32, 0);
                    self.current_sf2_program[ch_idx] = program;
                }
                s.note_on(ch as i32, note as i32, vel as i32);
            }
            self.note_routing[ch_idx][note_idx] = ROUTE_SF2;
        }
    }

    pub fn note_off(&mut self, ch: u8, note: u8) {
        let ch_idx = (ch & 0x0F) as usize;
        let note_idx = (note & 0x7F) as usize;
        
        match self.note_routing[ch_idx][note_idx] {
            ROUTE_PSG => {
                self.psg_synth.note_off(ch, note);
            }
            ROUTE_SF2 => {
                if let Some(s) = &mut self.sf2_synth {
                    s.note_off(ch as i32, note as i32);
                }
            }
            _ => {}
        }
        self.note_routing[ch_idx][note_idx] = ROUTE_NONE;
    }

    /// buf をモノラル PCM (f32) で埋める
    pub fn process_block(&mut self, buf: &mut [f32]) {
        let len = buf.len();
        
        // 1. SF2 波形を生成
        if let Some(s) = &mut self.sf2_synth {
            if self.left_buf.len() < len {
                self.left_buf.resize(len, 0.0);
                self.right_buf.resize(len, 0.0);
            }
            // render はバッファに加算ではなく上書きする
            s.render(&mut self.left_buf[..len], &mut self.right_buf[..len]);
        } else {
            if self.left_buf.len() < len {
                self.left_buf.resize(len, 0.0);
                self.right_buf.resize(len, 0.0);
            }
            self.left_buf[..len].fill(0.0);
            self.right_buf[..len].fill(0.0);
        }

        // 2. PSG 波形を生成
        if self.psg_buf.len() < len {
            self.psg_buf.resize(len, 0.0);
        }
        self.psg_buf[..len].fill(0.0); // 一応クリア
        self.psg_synth.process_block(&mut self.psg_buf[..len]);

        // 3. ミックスダウンして出力
        for i in 0..len {
            let sf2_mono = (self.left_buf[i] + self.right_buf[i]) * 0.5;
            let psg_mono = self.psg_buf[i];
            buf[i] = (sf2_mono + psg_mono).clamp(-1.0, 1.0);
        }
    }
}
