//! Standard MIDI File (SMF) パーサー
//! フォーマット 0 / 1 対応。SMPTE タイムコードは非対応。

use std::collections::HashMap;

// ─────────────────────────────────────────────
// 公開データ型
// ─────────────────────────────────────────────

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct NoteEvent {
    pub tick: u32,
    pub time_sec: f64,
    pub channel: u8,
    pub note: u8,
    pub velocity: u8,
    pub duration_tick: u32,
    pub duration_sec: f64,
    pub bank: u8,
    pub program: u8,
}

#[derive(Debug, Clone)]
pub struct MidiControlEvent {
    pub tick: u32,
    pub time_sec: f64,
    pub channel: u8,
    pub command: u8, // 0xB0, 0xC0, 0xE0 など
    pub data1: u8,
    pub data2: u8,
}

#[allow(dead_code)]
pub struct MidiData {
    pub format: u16,
    pub ticks_per_quarter: u32,
    pub notes: Vec<NoteEvent>,
    pub control_events: Vec<MidiControlEvent>,
    pub duration_sec: f64,
    /// (tick, マイクロ秒/拍) のリスト (tick 昇順)
    pub tempo_map: Vec<(u32, u32)>,
}

// ─────────────────────────────────────────────
// 内部型
// ─────────────────────────────────────────────

enum RawEventType {
    NoteOn { channel: u8, note: u8, velocity: u8 },
    NoteOff { channel: u8, note: u8 },
    ProgramChange { channel: u8, program: u8 },
    ControlChange { channel: u8, controller: u8, value: u8 },
    PitchBend { channel: u8, value: u16 },
    SetTempo { us_per_beat: u32 },
    Other,
}

struct RawEvent {
    tick: u32,
    event_type: RawEventType,
}

// ─────────────────────────────────────────────
// バイト列ユーティリティ
// ─────────────────────────────────────────────

fn read_u16_be(data: &[u8], offset: usize) -> Result<u16, String> {
    if offset + 2 > data.len() {
        return Err("Unexpected end reading u16".into());
    }
    Ok(u16::from_be_bytes([data[offset], data[offset + 1]]))
}

fn read_u32_be(data: &[u8], offset: usize) -> Result<u32, String> {
    if offset + 4 > data.len() {
        return Err("Unexpected end reading u32".into());
    }
    Ok(u32::from_be_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ]))
}

/// Variable-Length Quantity を読む
fn read_vlq(data: &[u8], offset: &mut usize) -> Result<u32, String> {
    let mut value: u32 = 0;
    for _ in 0..4 {
        if *offset >= data.len() {
            return Err("Unexpected end in VLQ".into());
        }
        let byte = data[*offset];
        *offset += 1;
        value = (value << 7) | (byte & 0x7F) as u32;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err("VLQ value too large".into())
}

// ─────────────────────────────────────────────
// イベント処理ヘルパー
// ─────────────────────────────────────────────

fn handle_channel_event(
    offset: &mut usize,
    data: &[u8],
    tick: u32,
    status: u8,
    events: &mut Vec<RawEvent>,
) -> Result<(), String> {
    let channel = status & 0x0F;
    match status & 0xF0 {
        0x90 => {
            // Note On
            if *offset + 2 > data.len() {
                return Err("Unexpected end in NoteOn".into());
            }
            let note = data[*offset];
            let velocity = data[*offset + 1];
            *offset += 2;
            if velocity == 0 {
                // velocity=0 は Note Off 扱い
                events.push(RawEvent { tick, event_type: RawEventType::NoteOff { channel, note } });
            } else {
                events.push(RawEvent {
                    tick,
                    event_type: RawEventType::NoteOn { channel, note, velocity },
                });
            }
        }
        0x80 => {
            // Note Off
            if *offset + 2 > data.len() {
                return Err("Unexpected end in NoteOff".into());
            }
            let note = data[*offset];
            *offset += 2; // release velocity は無視
            events.push(RawEvent { tick, event_type: RawEventType::NoteOff { channel, note } });
        }
        0xA0 | 0xE0 => {
            // Aftertouch / Pitch Bend
            if *offset + 2 > data.len() {
                return Err("Unexpected end in 2-byte event".into());
            }
            let d1 = data[*offset];
            let d2 = data[*offset + 1];
            *offset += 2;
            if (status & 0xF0) == 0xE0 {
                events.push(RawEvent { tick, event_type: RawEventType::PitchBend { channel, value: ((d2 as u16) << 7) | (d1 as u16) } });
            } else {
                events.push(RawEvent { tick, event_type: RawEventType::Other });
            }
        }
        0xB0 => {
            // Control Change (and Channel Mode)
            if *offset + 2 > data.len() {
                return Err("Unexpected end in CC".into());
            }
            let controller = data[*offset];
            let value = data[*offset + 1];
            *offset += 2;
            events.push(RawEvent { tick, event_type: RawEventType::ControlChange { channel, controller, value } });
        }
        0xC0 => {
            // Program Change
            if *offset + 1 > data.len() {
                return Err("Unexpected end in Program Change".into());
            }
            let program = data[*offset];
            *offset += 1;
            events.push(RawEvent { tick, event_type: RawEventType::ProgramChange { channel, program } });
        }
        0xD0 => {
            // Channel Pressure
            if *offset + 1 > data.len() {
                return Err("Unexpected end in 1-byte event".into());
            }
            *offset += 1;
            events.push(RawEvent { tick, event_type: RawEventType::Other });
        }
        _ => {
            // 不明なステータスは無視
        }
    }
    Ok(())
}

/// メタイベントを処理。End of Track なら true を返す。
fn handle_meta(
    offset: &mut usize,
    data: &[u8],
    tick: u32,
    events: &mut Vec<RawEvent>,
) -> Result<bool, String> {
    if *offset >= data.len() {
        return Err("Unexpected end in meta event type".into());
    }
    let meta_type = data[*offset];
    *offset += 1;

    let meta_len = read_vlq(data, offset)? as usize;

    if meta_type == 0x51 && meta_len == 3 {
        // Set Tempo
        if *offset + 3 > data.len() {
            return Err("Unexpected end in SetTempo".into());
        }
        let us = ((data[*offset] as u32) << 16)
            | ((data[*offset + 1] as u32) << 8)
            | (data[*offset + 2] as u32);
        events.push(RawEvent {
            tick,
            event_type: RawEventType::SetTempo { us_per_beat: us },
        });
    }

    if *offset + meta_len > data.len() {
        return Err("Meta event data exceeds track size".into());
    }
    *offset += meta_len;

    Ok(meta_type == 0x2F) // 0x2F = End of Track
}

// ─────────────────────────────────────────────
// トラック解析
// ─────────────────────────────────────────────

fn parse_track(data: &[u8]) -> Result<Vec<RawEvent>, String> {
    let mut events = Vec::new();
    let mut offset = 0usize;
    let mut current_tick: u32 = 0;
    let mut running_status: u8 = 0;

    while offset < data.len() {
        let delta = read_vlq(data, &mut offset)?;
        current_tick = current_tick.saturating_add(delta);

        if offset >= data.len() {
            break;
        }

        let byte = data[offset];

        if byte == 0xFF {
            // メタイベント → ランニングステータスをリセット
            offset += 1;
            let is_eot = handle_meta(&mut offset, data, current_tick, &mut events)?;
            running_status = 0;
            if is_eot {
                break;
            }
        } else if byte == 0xF0 || byte == 0xF7 {
            // SysEx → スキップ
            offset += 1;
            let len = read_vlq(data, &mut offset)? as usize;
            if offset + len > data.len() {
                return Err("SysEx data exceeds track size".into());
            }
            offset += len;
            running_status = 0;
        } else if byte & 0x80 != 0 {
            // 新しいチャンネルステータスバイト
            running_status = byte;
            offset += 1;
            handle_channel_event(&mut offset, data, current_tick, running_status, &mut events)?;
        } else {
            // ランニングステータス (データバイトのみ)
            if running_status != 0 {
                handle_channel_event(&mut offset, data, current_tick, running_status, &mut events)?;
            } else {
                offset += 1; // 無効バイトをスキップ
            }
        }
    }

    Ok(events)
}

// ─────────────────────────────────────────────
// Tick → 秒変換
// ─────────────────────────────────────────────

pub fn ticks_to_sec(tick: u32, tempo_map: &[(u32, u32)], tpq: u32) -> f64 {
    if tpq == 0 {
        return 0.0;
    }
    let mut time_sec = 0.0f64;
    let mut prev_tick: u32 = 0;
    let mut current_tempo: u32 = 500_000; // デフォルト 120 BPM

    for &(change_tick, new_tempo) in tempo_map {
        if change_tick >= tick {
            break;
        }
        let ticks = (change_tick - prev_tick) as f64;
        time_sec += ticks / tpq as f64 * current_tempo as f64 / 1_000_000.0;
        prev_tick = change_tick;
        current_tempo = new_tempo;
    }

    let remaining = (tick - prev_tick) as f64;
    time_sec += remaining / tpq as f64 * current_tempo as f64 / 1_000_000.0;
    time_sec
}

// ─────────────────────────────────────────────
// 公開 API
// ─────────────────────────────────────────────

pub fn parse(data: &[u8]) -> Result<MidiData, String> {
    // ヘッダー検証
    if data.len() < 14 {
        return Err("File too short to be a MIDI file".into());
    }
    if &data[0..4] != b"MThd" {
        return Err("Not a MIDI file (missing MThd header)".into());
    }

    let header_len = read_u32_be(data, 4)? as usize;
    let format = read_u16_be(data, 8)?;
    let num_tracks = read_u16_be(data, 10)?;
    let division = read_u16_be(data, 12)?;

    if division & 0x8000 != 0 {
        return Err("SMPTE time code division is not supported".into());
    }
    let tpq = division as u32;
    if tpq == 0 {
        return Err("Invalid ticks-per-quarter-note (0)".into());
    }

    // 全トラックのイベントを収集
    let mut all_events: Vec<RawEvent> = Vec::new();
    let mut offset = 8 + header_len;

    for _ in 0..num_tracks {
        if offset + 8 > data.len() {
            break;
        }
        if &data[offset..offset + 4] != b"MTrk" {
            return Err("Expected MTrk chunk".into());
        }
        let track_len = read_u32_be(data, offset + 4)? as usize;
        offset += 8;
        if offset + track_len > data.len() {
            return Err("Track data exceeds file size".into());
        }
        let track_events = parse_track(&data[offset..offset + track_len])?;
        all_events.extend(track_events);
        offset += track_len;
    }

    // 同一 tick 内: NoteOff → SetTempo → NoteOn の順でソート
    all_events.sort_by(|a, b| {
        let priority = |e: &RawEventType| match e {
            RawEventType::NoteOff { .. } => 0u8,
            RawEventType::SetTempo { .. } => 1,
            RawEventType::ProgramChange { .. } => 2,
            RawEventType::ControlChange { .. } => 3,
            RawEventType::PitchBend { .. } => 3,
            RawEventType::NoteOn { .. } => 4,
            RawEventType::Other => 5,
        };
        a.tick
            .cmp(&b.tick)
            .then(priority(&a.event_type).cmp(&priority(&b.event_type)))
    });

    // テンポマップ構築 (tick 0 のデフォルト含む)
    let mut tempo_map: Vec<(u32, u32)> = vec![(0, 500_000)];
    for ev in &all_events {
        if let RawEventType::SetTempo { us_per_beat } = ev.event_type {
            if ev.tick == 0 {
                tempo_map[0] = (0, us_per_beat);
            } else {
                tempo_map.push((ev.tick, us_per_beat));
            }
        }
    }
    tempo_map.sort_by_key(|&(tick, _)| tick);

    // NoteOn ↔ NoteOff マッチング → NoteEvent 生成
    // キー: (channel, note), 値: Vec<(start_tick, velocity, bank, program)>
    let mut pending: HashMap<(u8, u8), Vec<(u32, u8, u8, u8)>> = HashMap::new();
    let mut note_events: Vec<NoteEvent> = Vec::new();
    let mut control_events: Vec<MidiControlEvent> = Vec::new();
    
    let mut current_bank = [0u8; 16];
    let mut current_program = [0u8; 16];

    for ev in &all_events {
        match &ev.event_type {
            RawEventType::ControlChange { channel, controller: 0, value } => {
                current_bank[(*channel & 0x0F) as usize] = *value;
                control_events.push(MidiControlEvent {
                    tick: ev.tick,
                    time_sec: ticks_to_sec(ev.tick, &tempo_map, tpq),
                    channel: *channel,
                    command: 0xB0,
                    data1: 0,
                    data2: *value,
                });
            }
            RawEventType::ControlChange { channel, controller, value } => {
                control_events.push(MidiControlEvent {
                    tick: ev.tick,
                    time_sec: ticks_to_sec(ev.tick, &tempo_map, tpq),
                    channel: *channel,
                    command: 0xB0,
                    data1: *controller,
                    data2: *value,
                });
            }
            RawEventType::ProgramChange { channel, program } => {
                current_program[(*channel & 0x0F) as usize] = *program;
                control_events.push(MidiControlEvent {
                    tick: ev.tick,
                    time_sec: ticks_to_sec(ev.tick, &tempo_map, tpq),
                    channel: *channel,
                    command: 0xC0,
                    data1: *program,
                    data2: 0,
                });
            }
            RawEventType::PitchBend { channel, value } => {
                control_events.push(MidiControlEvent {
                    tick: ev.tick,
                    time_sec: ticks_to_sec(ev.tick, &tempo_map, tpq),
                    channel: *channel,
                    command: 0xE0,
                    data1: (*value & 0x7F) as u8,
                    data2: (*value >> 7) as u8,
                });
            }
            RawEventType::NoteOn { channel, note, velocity } => {
                let ch_idx = (*channel & 0x0F) as usize;
                pending
                    .entry((*channel, *note))
                    .or_default()
                    .push((ev.tick, *velocity, current_bank[ch_idx], current_program[ch_idx]));
            }
            RawEventType::NoteOff { channel, note } => {
                if let Some(stack) = pending.get_mut(&(*channel, *note)) {
                    if !stack.is_empty() {
                        let (start_tick, velocity, bank, program) = stack.remove(0); // FIFO
                        let time_sec = ticks_to_sec(start_tick, &tempo_map, tpq);
                        let end_sec = ticks_to_sec(ev.tick, &tempo_map, tpq);
                        note_events.push(NoteEvent {
                            tick: start_tick,
                            time_sec,
                            channel: *channel,
                            note: *note,
                            velocity,
                            duration_tick: ev.tick.saturating_sub(start_tick),
                            duration_sec: (end_sec - time_sec).max(0.0),
                            bank,
                            program,
                        });
                    }
                }
            }
            _ => {}
        }
    }

    // 対応する NoteOff がなかった NoteOn に最小デュレーションを付与
    for ((channel, note), stack) in pending {
        for (start_tick, velocity, bank, program) in stack {
            let time_sec = ticks_to_sec(start_tick, &tempo_map, tpq);
            note_events.push(NoteEvent {
                tick: start_tick,
                time_sec,
                channel,
                note,
                velocity,
                duration_tick: tpq / 4,
                duration_sec: 0.125,
                bank,
                program,
            });
        }
    }

    note_events.sort_by_key(|n| n.tick);

    let duration_sec = note_events
        .iter()
        .map(|n| n.time_sec + n.duration_sec)
        .fold(0.0f64, f64::max)
        + 1.5; // 減衰のための末尾余白

    Ok(MidiData {
        format,
        ticks_per_quarter: tpq,
        notes: note_events,
        control_events,
        duration_sec,
        tempo_map,
    })
}

// ─────────────────────────────────────────────
// テスト
// ─────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── テスト用 MIDI バイナリ生成ヘルパー ─────────

    /// 任意フォーマットの SMF を構築する
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

    /// SetTempo (120 BPM) + 1 拍の NoteOn/NoteOff (フォーマット 0)
    fn format0_one_note() -> Vec<u8> {
        let track = vec![
            // delta=0, SetTempo 500000 (120 BPM)
            0x00, 0xFF, 0x51, 0x03, 0x07, 0xA1, 0x20,
            // delta=0, NoteOn ch=0 note=60(C4) vel=100
            0x00, 0x90, 0x3C, 0x64,
            // delta=96 ticks (= 1 beat = 0.5s), NoteOff ch=0 note=60
            0x60, 0x80, 0x3C, 0x00,
            // delta=0, End of Track
            0x00, 0xFF, 0x2F, 0x00,
        ];
        make_midi(0, 96, &[track])
    }

    /// フォーマット 1: テンポトラック + ノートトラック
    fn format1_one_note() -> Vec<u8> {
        let tempo_track = vec![
            0x00, 0xFF, 0x51, 0x03, 0x07, 0xA1, 0x20, // SetTempo 500000
            0x00, 0xFF, 0x2F, 0x00,                    // End of Track
        ];
        let note_track = vec![
            0x00, 0x90, 0x3C, 0x64, // NoteOn ch0 C4 vel=100
            0x60, 0x80, 0x3C, 0x00, // NoteOff (delta=96)
            0x00, 0xFF, 0x2F, 0x00, // End of Track
        ];
        make_midi(1, 96, &[tempo_track, note_track])
    }

    /// ドラムチャンネル (ch9) を含む最小 MIDI
    fn format0_with_drums() -> Vec<u8> {
        let track = vec![
            0x00, 0xFF, 0x51, 0x03, 0x07, 0xA1, 0x20, // SetTempo
            // NoteOn ch=9(drums) note=36(BassDrum) vel=127
            0x00, 0x99, 0x24, 0x7F,
            // delta=24, NoteOff ch=9 note=36
            0x18, 0x89, 0x24, 0x00,
            // NoteOn ch=9 note=38(Snare) vel=100
            0x18, 0x99, 0x26, 0x64,
            // delta=24, NoteOff ch=9 note=38
            0x18, 0x89, 0x26, 0x00,
            0x00, 0xFF, 0x2F, 0x00, // End of Track
        ];
        make_midi(0, 96, &[track])
    }

    // ── 基本パース ──────────────────────────────

    #[test]
    fn test_parse_format0_one_note() {
        let data   = format0_one_note();
        let result = parse(&data).expect("parse should succeed");

        assert_eq!(result.format, 0);
        assert_eq!(result.ticks_per_quarter, 96);
        assert_eq!(result.notes.len(), 1);

        let n = &result.notes[0];
        assert_eq!(n.channel,  0);
        assert_eq!(n.note,     60);   // C4
        assert_eq!(n.velocity, 100);
        assert_eq!(n.tick,     0);
    }

    #[test]
    fn test_parse_format1_merges_tracks() {
        let data   = format1_one_note();
        let result = parse(&data).expect("parse should succeed");

        assert_eq!(result.format, 1);
        // テンポトラックにノートはなく、ノートトラックの 1 音だけが含まれる
        assert_eq!(result.notes.len(), 1);
        assert_eq!(result.notes[0].note, 60);
    }

    // ── タイミング計算 ────────────────────────────

    #[test]
    fn test_note_timing_at_120bpm() {
        // tpq=96, tempo=500000 us/beat (120 BPM)
        // 1 ビート = 96 tick = 0.5 s
        let tempo_map = vec![(0u32, 500_000u32)];
        let tpq = 96;

        let t0 = ticks_to_sec(0,  &tempo_map, tpq);
        let t1 = ticks_to_sec(96, &tempo_map, tpq);
        let t2 = ticks_to_sec(48, &tempo_map, tpq);

        assert!((t0 - 0.0).abs() < 1e-9,  "tick 0 → 0.0 s");
        assert!((t1 - 0.5).abs() < 1e-6,  "tick 96 → 0.5 s");
        assert!((t2 - 0.25).abs() < 1e-6, "tick 48 → 0.25 s");
    }

    #[test]
    fn test_note_duration_is_correct() {
        let data   = format0_one_note();
        let result = parse(&data).expect("parse");
        let n      = &result.notes[0];

        // 開始時刻 0 s, 長さ 1 拍 = 0.5 s
        assert!((n.time_sec - 0.0).abs() < 1e-6);
        assert!((n.duration_sec - 0.5).abs() < 0.01,
            "1 拍の長さが 0.5 秒であるはず (actual: {})", n.duration_sec);
    }

    #[test]
    fn test_tempo_change_mid_song() {
        // 最初 120 BPM → tick96 で 60 BPM に変更
        let track = vec![
            // SetTempo 500000 (120 BPM) at tick 0
            0x00, 0xFF, 0x51, 0x03, 0x07, 0xA1, 0x20,
            // NoteOn at tick 0
            0x00, 0x90, 0x3C, 0x64,
            // NoteOff at tick 96  (= 0.5 s with 120 BPM)
            0x60, 0x80, 0x3C, 0x00,
            // SetTempo 1000000 (60 BPM) at tick 96
            0x00, 0xFF, 0x51, 0x03, 0x0F, 0x42, 0x40,
            // NoteOn at tick 96
            0x00, 0x90, 0x40, 0x64,
            // NoteOff at tick 192 (= 1.0 s with 60 BPM after tick96)
            0x60, 0x80, 0x40, 0x00,
            0x00, 0xFF, 0x2F, 0x00,
        ];
        let data   = make_midi(0, 96, &[track]);
        let result = parse(&data).expect("parse");

        assert_eq!(result.notes.len(), 2);
        let n0 = &result.notes[0];
        let n1 = &result.notes[1];
        // 第1音: 0 s 開始、0.5 s の長さ
        assert!((n0.time_sec - 0.0).abs() < 1e-6);
        assert!((n0.duration_sec - 0.5).abs() < 0.01);
        // 第2音: 0.5 s 開始 (1ビート目が 0.5s)、1.0 s の長さ (60BPM)
        assert!((n1.time_sec - 0.5).abs() < 0.01);
        assert!((n1.duration_sec - 1.0).abs() < 0.02);
    }

    // ── ランニングステータス ───────────────────────

    #[test]
    fn test_running_status_noteon() {
        // 2 つ目の NoteOn はランニングステータス (0x90 省略)
        let track = vec![
            0x00, 0x90, 0x3C, 0x64, // NoteOn ch0 C4
            0x00, 0x3E, 0x50,       // NoteOn ch0 D4 (running status)
            0x60, 0x80, 0x3C, 0x00, // NoteOff C4 (delta=96)
            0x60, 0x3E, 0x00,       // NoteOff D4 (running status 0x80)
            0x00, 0xFF, 0x2F, 0x00,
        ];
        let data   = make_midi(0, 96, &[track]);
        let result = parse(&data).expect("parse");

        assert_eq!(result.notes.len(), 2, "ランニングステータスで 2 音とれるはず");
        let notes: Vec<u8> = result.notes.iter().map(|n| n.note).collect();
        assert!(notes.contains(&60), "C4 が含まれるはず");
        assert!(notes.contains(&62), "D4 が含まれるはず");
    }

    // ── velocity=0 は NoteOff 扱い ────────────────

    #[test]
    fn test_noteon_velocity0_treated_as_noteoff() {
        let track = vec![
            0x00, 0x90, 0x3C, 0x64, // NoteOn
            0x60, 0x90, 0x3C, 0x00, // NoteOn vel=0 → NoteOff と同義
            0x00, 0xFF, 0x2F, 0x00,
        ];
        let data   = make_midi(0, 96, &[track]);
        let result = parse(&data).expect("parse");

        assert_eq!(result.notes.len(), 1);
        // 96 ticks = 0.5 s のデュレーション
        assert!((result.notes[0].duration_sec - 0.5).abs() < 0.01);
    }

    // ── ドラムチャンネル ──────────────────────────

    #[test]
    fn test_drum_channel_parsed_as_ch9() {
        let data   = format0_with_drums();
        let result = parse(&data).expect("parse");

        // すべてのノートがチャンネル 9 のはず
        assert!(result.notes.iter().all(|n| n.channel == 9),
            "ドラムノートは ch=9 であるはず");
        // バスドラムとスネアの 2 音
        assert_eq!(result.notes.len(), 2);
    }

    // ── エラーケース ─────────────────────────────

    #[test]
    fn test_invalid_header_returns_error() {
        let result = parse(b"RIFF\x00\x00\x00\x06\x00\x00\x00\x01\x00\x60");
        assert!(result.is_err(), "無効なヘッダーはエラーになるはず");
    }

    #[test]
    fn test_too_short_returns_error() {
        let result = parse(b"MThd");
        assert!(result.is_err());
    }

    #[test]
    fn test_smpte_timecode_returns_error() {
        // division の最上位ビットが 1 = SMPTE (非サポート)
        let mut data = format0_one_note();
        data[12] = 0xE7; // tpq の上位バイトを 0x80 以上に変更
        let result = parse(&data);
        assert!(result.is_err(), "SMPTE タイムコードはエラーになるはず");
    }

    // ── ticks_to_sec 数値検証 ───────────────────────

    #[test]
    fn test_ticks_to_sec_default_tempo() {
        // テンポマップなし = デフォルト 500000 us/beat
        let tm  = vec![(0u32, 500_000u32)];
        let tpq = 480u32;
        // 480 ticks = 1 beat = 0.5 s
        let sec = ticks_to_sec(480, &tm, tpq);
        assert!((sec - 0.5).abs() < 1e-9);
    }

    #[test]
    fn test_ticks_to_sec_zero_tpq_returns_zero() {
        let tm = vec![(0u32, 500_000u32)];
        assert_eq!(ticks_to_sec(100, &tm, 0), 0.0);
    }
}
