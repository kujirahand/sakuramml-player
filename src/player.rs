//! 再生状態管理 + ストリーミング (チャンク) レンダリング
//!
//! ## 設計
//! - `load()` : MIDI を解析し、サンプル単位のイベントリストを構築してカーソルを先頭に設定
//! - `seek_to()` : 任意位置にカーソルを移動、その時点で発音中のノートをシンセに再ロード
//! - `render_next()` : カーソル位置から `frames` サンプル分を PCM に変換して返す
//! - `is_render_done()` : 全サンプルを出力し終えたか
//!
//! JS 側は `render_next()` を 10 秒分ずつ呼び出し、
//! `AudioBufferSourceNode` を時差スケジューリングすることで
//! メモリを節約しながらシームレスな再生を実現する。

use crate::midi_parser::{parse, MidiData, NoteEvent};
use crate::synth::Synth;

// ─────────────────────────────────────────────
// チャンクイベント (Copy で借用問題を回避)
// ─────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
enum ChunkEventType {
    NoteOn { note: u8, vel: u8, bank: u8, program: u8 },
    NoteOff { note: u8 },
    Control { command: u8, data1: u8, data2: u8 },
}

impl ChunkEventType {
    fn priority(&self) -> u8 {
        match self {
            ChunkEventType::Control { .. } => 0,
            ChunkEventType::NoteOff { .. } => 1,
            ChunkEventType::NoteOn { .. } => 2,
        }
    }
}

#[derive(Clone, Copy)]
struct ChunkEvent {
    sample: usize,
    ch: u8,
    event: ChunkEventType,
}

// ─────────────────────────────────────────────
// Player 構造体
// ─────────────────────────────────────────────

pub struct Player {
    sr: f32,
    midi_data: Option<MidiData>,
    synth: Synth,

    /// load() 時に構築されるサンプル単位イベントリスト
    events: Vec<ChunkEvent>,
    /// 次に処理するイベントのインデックス
    ev_idx: usize,
    /// 次に生成するサンプルの位置
    pub pos_sample: usize,
    /// 総サンプル数 (duration + 末尾余白)
    pub total_samples: usize,
}

impl Player {
    pub fn new(sr: f32) -> Self {
        Player {
            sr,
            midi_data: None,
            synth: Synth::new(sr),
            events: Vec::new(),
            ev_idx: 0,
            pos_sample: 0,
            total_samples: 0,
        }
    }

    // ─────────────────────────────────────────
    // 公開 API
    // ─────────────────────────────────────────

    pub fn load(&mut self, data: &[u8]) -> Result<String, String> {
        let midi = parse(data)?;
        let json = Self::data_to_json(&midi.notes, &midi.beats, &midi.texts);

        // サンプル単位のイベントリストを構築
        let sr = self.sr as f64;
        let total = (midi.duration_sec * sr).ceil() as usize + 1;

        let mut evs: Vec<ChunkEvent> = Vec::with_capacity(midi.notes.len() * 2 + midi.control_events.len());
        for n in &midi.notes {
            let on_s  = (n.time_sec * sr) as usize;
            let off_s = ((n.time_sec + n.duration_sec) * sr) as usize;
            evs.push(ChunkEvent { sample: on_s, ch: n.channel, event: ChunkEventType::NoteOn { note: n.note, vel: n.velocity, bank: n.bank, program: n.program } });
            evs.push(ChunkEvent { sample: off_s.min(total-1), ch: n.channel, event: ChunkEventType::NoteOff { note: n.note } });
        }
        for c in &midi.control_events {
            let s = (c.time_sec * sr) as usize;
            evs.push(ChunkEvent { sample: s.min(total-1), ch: c.channel, event: ChunkEventType::Control { command: c.command, data1: c.data1, data2: c.data2 } });
        }
        // 同一サンプル内: Control -> NoteOff -> NoteOn の順
        evs.sort_by(|a, b| a.sample.cmp(&b.sample).then(a.event.priority().cmp(&b.event.priority())));

        self.events         = evs;
        self.total_samples  = total;
        self.midi_data      = Some(midi);

        // カーsorを先頭に初期化
        self.seek_internal(0);

        Ok(json)
    }

    pub fn get_duration(&self) -> f64 {
        self.midi_data.as_ref().map(|d| d.duration_sec).unwrap_or(0.0)
    }

    pub fn get_note_events_json(&self) -> String {
        match &self.midi_data {
            Some(d) => Self::data_to_json(&d.notes, &d.beats, &d.texts),
            None    => r#"{"notes":[],"beats":[],"texts":[]}"#.to_string(),
        }
    }

    pub fn get_total_samples(&self) -> usize {
        self.total_samples
    }

    /// 再生カーソルを `time_sec` 秒へ移動。
    /// その時点で発音中のノートをシンセに即時ロードする。
    pub fn seek_to(&mut self, time_sec: f64) {
        let pos = (time_sec.max(0.0) * self.sr as f64) as usize;
        let pos = pos.min(self.total_samples);

        // seek 位置で発音中のノートを収集 (借用を先に終わらせる)
        let mut active_notes = Vec::new();
        let mut latest_controls = std::collections::HashMap::new();

        if let Some(midi) = &self.midi_data {
            let sr = self.sr as f64;
            for n in &midi.notes {
                let on_s  = (n.time_sec * sr) as usize;
                let off_s = ((n.time_sec + n.duration_sec) * sr) as usize;
                if on_s <= pos && pos < off_s {
                    active_notes.push((n.channel, n.note, n.velocity, n.bank, n.program));
                }
            }
            for c in &midi.control_events {
                let s = (c.time_sec * sr) as usize;
                if s <= pos {
                    latest_controls.insert((c.channel, c.command, c.data1), c.data2);
                }
            }
        }

        self.synth.reset();
        
        let mut controls: Vec<_> = latest_controls.into_iter().collect();
        controls.sort_by_key(|(k, _)| k.1); // コマンド順 (0xB0 -> 0xC0 -> 0xE0)
        for ((ch, command, data1), data2) in controls {
            self.synth.control(ch, command, data1, data2);
        }

        for (ch, note, vel, bank, program) in active_notes {
            self.synth.note_on(ch, note, vel, bank, program);
        }

        self.seek_internal(pos);
    }

    /// カーソル位置から `frames` サンプル分をインターリーブされたステレオ PCM (f32) に変換して返す。
    /// (返り値の `Vec` の長さは `frames * 2` になる)
    /// 末尾に達した場合は空の Vec を返す。
    pub fn render_next(&mut self, frames: usize) -> Vec<f32> {
        if self.pos_sample >= self.total_samples {
            return Vec::new();
        }

        let end           = (self.pos_sample + frames).min(self.total_samples);
        let actual_frames = end - self.pos_sample;
        let mut output    = vec![0.0f32; actual_frames * 2];

        const BLOCK: usize = 128;
        let mut cur = 0usize;

        while cur < actual_frames {
            let blk_end     = (cur + BLOCK).min(actual_frames);
            let abs_blk_end = self.pos_sample + blk_end;

            // このブロックに含まれるイベントを発火
            while self.ev_idx < self.events.len() && self.events[self.ev_idx].sample < abs_blk_end {
                let ev = self.events[self.ev_idx]; // Copy
                match ev.event {
                    ChunkEventType::NoteOn { note, vel, bank, program } => {
                        self.synth.note_on(ev.ch, note, vel, bank, program);
                    }
                    ChunkEventType::NoteOff { note } => {
                        self.synth.note_off(ev.ch, note);
                    }
                    ChunkEventType::Control { command, data1, data2 } => {
                        self.synth.control(ev.ch, command, data1, data2);
                    }
                }
                self.ev_idx += 1;
            }

            self.synth.process_block(&mut output[cur * 2 .. blk_end * 2]);
            cur = blk_end;
        }

        self.pos_sample = end;
        output
    }

    /// 全サンプルを出力し終えたか
    pub fn is_render_done(&self) -> bool {
        self.pos_sample >= self.total_samples
    }

    // ─────────────────────────────────────────
    // 内部ヘルパー
    // ─────────────────────────────────────────

    /// シンセをリセットせずにカーソル位置だけ移動する内部メソッド
    fn seek_internal(&mut self, pos: usize) {
        self.ev_idx    = self.events.partition_point(|e| e.sample < pos);
        self.pos_sample = pos;
        self.synth.reset();
    }

    fn data_to_json(notes: &[NoteEvent], beats: &[crate::midi_parser::BeatInfo], texts: &[crate::midi_parser::MidiTextEvent]) -> String {
        let note_items: Vec<String> = notes.iter().map(|n| {
            format!(
                r#"{{"tick":{tick},"time":{time:.4},"ch":{ch},"note":{note},"vel":{vel},"dur":{dur:.4}}}"#,
                tick = n.tick,
                time = n.time_sec,
                ch   = n.channel,
                note = n.note,
                vel  = n.velocity,
                dur  = n.duration_sec,
            )
        }).collect();
        let beat_items: Vec<String> = beats.iter().map(|b| {
            format!(
                r#"{{"time":{time:.4},"is_measure":{measure}}}"#,
                time = b.time_sec,
                measure = if b.is_measure { "true" } else { "false" }
            )
        }).collect();
        let text_items: Vec<String> = texts.iter().map(|t| {
            // エスケープ処理 (簡易的)
            let safe_text = t.text.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n").replace('\r', "\\r");
            format!(
                r#"{{"time":{time:.4},"type":{ttype},"text":"{text}"}}"#,
                time = t.time_sec,
                ttype = t.text_type,
                text = safe_text
            )
        }).collect();
        format!(
            r#"{{"notes":[{notes}],"beats":[{beats}],"texts":[{texts}]}}"#,
            notes = note_items.join(","),
            beats = beat_items.join(","),
            texts = text_items.join(",")
        )
    }
}

// ─────────────────────────────────────────────
// テスト
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 44100.0;
    const CHUNK: usize = 44100 * 10; // 10 秒分

    // ── テスト用 MIDI バイナリ生成ヘルパー ────────

    fn make_midi(format: u16, tpq: u16, tracks: &[Vec<u8>]) -> Vec<u8> {
        let mut d = Vec::new();
        d.extend_from_slice(b"MThd");
        d.extend_from_slice(&6u32.to_be_bytes());
        d.extend_from_slice(&format.to_be_bytes());
        d.extend_from_slice(&(tracks.len() as u16).to_be_bytes());
        d.extend_from_slice(&tpq.to_be_bytes());
        for t in tracks {
            d.extend_from_slice(b"MTrk");
            d.extend_from_slice(&(t.len() as u32).to_be_bytes());
            d.extend_from_slice(t);
        }
        d
    }

    /// 120BPM, tpq=96, 1 拍の C4 ノート
    fn midi_one_note() -> Vec<u8> {
        let track = vec![
            0x00, 0xFF, 0x51, 0x03, 0x07, 0xA1, 0x20, // SetTempo 120BPM
            0x00, 0x90, 0x3C, 0x64,                    // NoteOn C4
            0x60, 0x80, 0x3C, 0x00,                    // NoteOff (delta=96)
            0x00, 0xFF, 0x2F, 0x00,                    // End of Track
        ];
        make_midi(0, 96, &[track])
    }

    /// ch=9 ドラム (BassDrum + Snare) を含む MIDI
    fn midi_with_drums() -> Vec<u8> {
        let track = vec![
            0x00, 0xFF, 0x51, 0x03, 0x07, 0xA1, 0x20, // tempo
            0x00, 0xB9, 0x00, 100,  // CC#0, Bank=100
            0x00, 0x99, 0x24, 0x7F, // BassDrum
            0x18, 0x89, 0x24, 0x00,
            0x18, 0x99, 0x26, 0x64, // Snare
            0x18, 0x89, 0x26, 0x00,
            0x00, 0xFF, 0x2F, 0x00,
        ];
        make_midi(0, 96, &[track])
    }

    // ── ロードなし ────────────────────────────

    #[test]
    fn test_empty_player_render_returns_empty() {
        let mut p = Player::new(SR);
        let buf = p.render_next(CHUNK);
        assert!(buf.is_empty(), "ロードなしは空の Vec を返すはず");
    }

    #[test]
    fn test_is_render_done_before_load() {
        let p = Player::new(SR);
        // ロード前は total_samples=0, pos=0 なので done=true
        assert!(p.is_render_done());
    }

    // ── ロード後の基本動作 ────────────────────

    #[test]
    fn test_load_returns_valid_json() {
        let mut p   = Player::new(SR);
        let json = p.load(&midi_one_note()).expect("load");
        // JSON オブジェクト形式の確認
        assert!(json.starts_with('{'), "JSON は '{{' 始まり");
        assert!(json.ends_with('}'),  "JSON は '}}' 終わり");
        // notes, beats が含まれる
        assert!(json.contains("\"notes\":["));
        assert!(json.contains("\"beats\":["));
    }

    #[test]
    fn test_get_duration_after_load() {
        let mut p = Player::new(SR);
        p.load(&midi_one_note()).expect("load");
        let dur = p.get_duration();
        // 0.5 s のノート + 減衰余白 1.5 s = 約 2.0 s以上
        assert!(dur > 1.5, "曲の長さは 1.5 s 以上のはず (actual={})", dur);
    }

    #[test]
    fn test_total_samples_consistent_with_duration() {
        let mut p = Player::new(SR);
        p.load(&midi_one_note()).expect("load");
        let expected = (p.get_duration() * SR as f64).ceil() as usize + 1;
        // 1 サンプルの誤差許容
        assert!((p.total_samples as isize - expected as isize).abs() <= 1);
    }

    // ── チャンクレンダリング ──────────────────────

    #[test]
    fn test_render_next_returns_correct_length() {
        let mut p = Player::new(SR);
        p.load(&midi_one_note()).expect("load");
        let total = p.total_samples;

        // 1 回目の render_next は min(CHUNK, total) を返す (ステレオなので x2)
        let chunk = p.render_next(CHUNK);
        let expected_len = CHUNK.min(total) * 2;
        assert_eq!(chunk.len(), expected_len);
    }

    #[test]
    fn test_render_chunks_sum_equals_total_samples() {
        let mut p = Player::new(SR);
        p.load(&midi_one_note()).expect("load");
        let total = p.total_samples;

        let mut rendered = 0usize;
        while !p.is_render_done() {
            let chunk = p.render_next(CHUNK);
            assert!(!chunk.is_empty(), "未完了時に空が返ってはいけない");
            rendered += chunk.len() / 2;
        }

        assert_eq!(rendered, total, 
            "チャンクの合計サンプル数が total_samples と一致するはず (rendered={}, total={})",
            rendered, total);
    }

    #[test]
    fn test_render_returns_empty_after_done() {
        let mut p = Player::new(SR);
        p.load(&midi_one_note()).expect("load");
        // 全部レンダリング完了
        while !p.is_render_done() { p.render_next(CHUNK); }
        // その後は空返り
        assert!(p.render_next(CHUNK).is_empty(), "完了後は空の Vec を返すはず");
    }

    #[test]
    fn test_is_render_done_after_full_render() {
        let mut p = Player::new(SR);
        p.load(&midi_one_note()).expect("load");
        while !p.is_render_done() { p.render_next(CHUNK); }
        assert!(p.is_render_done());
    }

    // ── seek_to ────────────────────────────────────

    #[test]
    fn test_seek_to_zero_restarts_render() {
        let mut p = Player::new(SR);
        p.load(&midi_one_note()).expect("load");
        let total = p.total_samples;

        // 全部レンダリングしてから seek_to(0)
        while !p.is_render_done() { p.render_next(CHUNK); }
        assert!(p.is_render_done());

        p.seek_to(0.0);
        assert!(!p.is_render_done(), "seek_to(0) 後は再度未完了になるはず");
        assert_eq!(p.pos_sample, 0);

        // 再レンダリングも同じ合計サンプル数
        let mut rendered = 0;
        while !p.is_render_done() { rendered += p.render_next(CHUNK).len() / 2; }
        assert_eq!(rendered, total);
    }

    #[test]
    fn test_seek_to_middle_reduces_remaining() {
        let mut p = Player::new(SR);
        p.load(&midi_one_note()).expect("load");
        let total = p.total_samples;

        let seek_s = p.get_duration() / 2.0;
        p.seek_to(seek_s);

        let mid_sample = (seek_s * SR as f64) as usize;
        assert!((p.pos_sample as isize - mid_sample as isize).abs() <= 1,
            "カーソルが期待位置の近くにあるはず (pos={}, expected~={})",
            p.pos_sample, mid_sample);
        assert!(p.pos_sample < total);
    }

    #[test]
    fn test_seek_beyond_duration_marks_done() {
        let mut p = Player::new(SR);
        p.load(&midi_one_note()).expect("load");
        let total = p.total_samples;

        p.seek_to(9999.0); // 曲より大きな値
        assert_eq!(p.pos_sample, total, "曲末尾にクランプされるはず");
        assert!(p.is_render_done());
    }

    // ── ドラム ──────────────────────────────────────

    #[test]
    fn test_render_with_drums_no_panic() {
        let mut p = Player::new(SR);
        p.load(&midi_with_drums()).expect("load");
        // ドラム入り MIDI がパニックせずにレンダリングできる
        while !p.is_render_done() { p.render_next(CHUNK); }
    }

    #[test]
    fn test_drum_render_produces_audio() {
        let mut p = Player::new(SR);
        p.load(&midi_with_drums()).expect("load");
        let chunk = p.render_next(CHUNK);
        let max = chunk.iter().map(|x| x.abs()).fold(0.0f32, f32::max);
        assert!(max > 0.0, "ドラム入り MIDI の PCM は非ゼロのはず (max={})", max);
    }
}
