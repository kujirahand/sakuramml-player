use std::env;
use std::fs;
use wav_io;
use sakuramml_player::player::Player;
use sakuramml_player::soundfont;

#[cfg(not(target_arch = "wasm32"))]
use rodio::{
    cpal::traits::{DeviceTrait, HostTrait},
    OutputStream,
    OutputStreamHandle,
    Sink,
    Source,
};

const CLI_CHUNK_DIVISOR: usize = 2;
const CLI_MIN_CHUNK_FRAMES: usize = 2048;

struct PlayerSource {
    player: Player,
    current_chunk: std::vec::IntoIter<f32>,
    sample_rate: u32,
    chunk_frames: usize,
}

impl PlayerSource {
    fn new(mut player: Player, sample_rate: u32) -> Self {
        let chunk_frames = cli_chunk_frames(sample_rate);
        let chunk = player.render_next(chunk_frames);
        Self {
            player,
            current_chunk: chunk.into_iter(),
            sample_rate,
            chunk_frames,
        }
    }
}

impl Iterator for PlayerSource {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(sample) = self.current_chunk.next() {
            return Some(sample);
        }
        
        if self.player.is_render_done() {
            return None;
        }

        let chunk = self.player.render_next(self.chunk_frames);
        if chunk.is_empty() {
            return None;
        }
        self.current_chunk = chunk.into_iter();
        self.current_chunk.next()
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Source for PlayerSource {
    fn current_frame_len(&self) -> Option<usize> {
        None
    }

    fn channels(&self) -> u16 {
        2
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn total_duration(&self) -> Option<std::time::Duration> {
        None
    }
}

fn cli_chunk_frames(sample_rate: u32) -> usize {
    ((sample_rate as usize) / CLI_CHUNK_DIVISOR).max(CLI_MIN_CHUNK_FRAMES)
}

#[cfg(not(target_arch = "wasm32"))]
fn open_output_stream() -> Result<(OutputStream, OutputStreamHandle, u32), String> {
    let host = rodio::cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| "既定のオーディオ出力デバイスが見つかりません。".to_string())?;
    let config = device
        .default_output_config()
        .map_err(|e| format!("オーディオ出力設定の取得に失敗しました: {}", e))?;
    let sample_rate = config.sample_rate().0;
    let (stream, stream_handle) = OutputStream::try_from_device_config(&device, config)
        .map_err(|e| format!("オーディオ出力デバイスの初期化に失敗しました: {}", e))?;

    Ok((stream, stream_handle, sample_rate))
}

fn meta_text_type_name(text_type: u8) -> &'static str {
    match text_type {
        0x01 => "Text",
        0x02 => "Copyright",
        0x03 => "TrackName",
        0x04 => "InstrumentName",
        0x05 => "Lyric",
        0x06 => "Marker",
        0x07 => "CuePoint",
        _ => "Unknown",
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn play_audio(
    midi_data: &[u8],
    sf2_data: Option<&[u8]>
) -> Result<(), String> {
    if let Some(data) = sf2_data {
        if let Err(e) = soundfont::load_soundfont_bytes(data) {
            eprintln!("SoundFontの解析に失敗しました: {:?}", e);
        }
    }

    let (_stream, stream_handle, sample_rate) = open_output_stream()?;
    let sample_rate = sample_rate as f32;
    let mut player = Player::new(sample_rate);
    if let Err(e) = player.load(midi_data) {
        return Err(format!("MIDIの解析に失敗しました: {}", e));
    }
    
    let sink = Sink::try_new(&stream_handle)
        .map_err(|e| format!("Sinkの作成に失敗しました: {}", e))?;

    let source = PlayerSource::new(player, sample_rate as u32);
    
    println!("♪ 再生を開始します... ({} Hz)", sample_rate as u32);
    sink.append(source);
    sink.sleep_until_end();
    println!("♪ 再生が完了しました。");

    Ok(())
}

/// MIDI データを WAV ファイルに書き出す
pub fn convert_midi_to_wav(
    midi_data: &[u8], 
    output_path: &str, 
    sf2_data: Option<&[u8]>
) -> Result<(), String> {
    if let Some(data) = sf2_data {
        if let Err(e) = soundfont::load_soundfont_bytes(data) {
            eprintln!("SoundFontの解析に失敗しました: {:?}", e);
        }
    }

    let sample_rate = 44100.0;
    let mut player = Player::new(sample_rate);
    if let Err(e) = player.load(midi_data) {
        return Err(format!("MIDIの解析に失敗しました: {}", e));
    }

    let total_samples = player.get_total_samples();
    let samples = player.render_next(total_samples as usize);

    let mut head = wav_io::new_stereo_header();
    head.sample_rate = sample_rate as u32;
    head.sample_format = wav_io::header::SampleFormat::Float;
    head.bits_per_sample = 32;
    
    let mut file_out = std::fs::File::create(output_path)
        .map_err(|e| format!("WAVファイルの作成に失敗しました: {}", e))?;

    wav_io::write_to_file(&mut file_out, &head, &samples)
        .map_err(|e| format!("WAVファイルの保存に失敗しました: {}", e))?;

    Ok(())
}

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("使い方:");
        eprintln!("  再生: {} <input.mid or input.mml>", args[0]);
        eprintln!("  WAV書き出し: {} <input.mid or input.mml> <output.wav>", args[0]);
        std::process::exit(1);
    }

    let input_path = &args[1];
    let output_path = if args.len() >= 3 {
        Some(args[2].clone())
    } else {
        None
    };

    println!("SoundFontを読み込んでいます...");
    let sf2_path = "www/fonts/TimGM6mb.sf2";
    let sf2_data = match fs::read(sf2_path) {
        Ok(data) => {
            println!("SoundFontの読み込みが完了しました: {}", sf2_path);
            Some(data)
        },
        Err(_) => {
            eprintln!("警告: {} を読み込めませんでした。一部の音が鳴らない場合があります。", sf2_path);
            eprintln!("（プロジェクトのルートディレクトリで実行してみてください）");
            None
        }
    };

    println!("入力を読み込んでいます: {}", input_path);
    let mut midi_data = fs::read(input_path).unwrap_or_else(|e| {
        eprintln!("ファイルの読み込みに失敗しました: {}", e);
        std::process::exit(1);
    });

    if input_path.to_lowercase().ends_with(".mml") {
        println!("MMLをコンパイルしています...");
        let mml_text = String::from_utf8_lossy(&midi_data).to_string();
        let res = sakuramml::compile(&mml_text, sakuramml::SAKURA_DEBUG_NONE);
        midi_data = res.bin;
    }

    if let Ok(midi) = sakuramml_player::midi_parser::parse(&midi_data) {
        if !midi.texts.is_empty() {
            println!("MetaText:");
            for text in &midi.texts {
                println!(
                    "  [{:.3}s] {}: {}",
                    text.time_sec,
                    meta_text_type_name(text.text_type),
                    text.text
                );
            }
        }
    }
    
    // Dump all raw meta events (using hacky search)
    for i in 0..midi_data.len() - 2 {
        if midi_data[i] == 0xFF {
            let meta_type = midi_data[i+1];
            let meta_len = midi_data[i+2];
            println!("Meta Event: type={}, len={}", meta_type, meta_len);
        }
    }

    if let Some(out_path) = output_path {
        println!("WAVファイルに書き出しています: {}", out_path);
        match convert_midi_to_wav(&midi_data, &out_path, sf2_data.as_deref()) {
            Ok(_) => println!("完了しました。WAVファイルを保存しました: {}", out_path),
            Err(e) => {
                eprintln!("エラー: {}", e);
                std::process::exit(1);
            }
        }
    } else {
        #[cfg(not(target_arch = "wasm32"))]
        {
            match play_audio(&midi_data, sf2_data.as_deref()) {
                Ok(_) => {},
                Err(e) => {
                    eprintln!("再生エラー: {}", e);
                    std::process::exit(1);
                }
            }
        }
        #[cfg(target_arch = "wasm32")]
        {
            eprintln!("エラー: WASM環境ではリアルタイム再生はコマンドラインから実行できません。");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// 1拍のC4ノートを持つ簡単なMIDIバイナリを作成するヘルパー
    fn make_dummy_midi() -> Vec<u8> {
        let mut d = Vec::new();
        d.extend_from_slice(b"MThd");
        d.extend_from_slice(&6u32.to_be_bytes()); // length
        d.extend_from_slice(&0u16.to_be_bytes()); // format 0
        d.extend_from_slice(&1u16.to_be_bytes()); // 1 track
        d.extend_from_slice(&96u16.to_be_bytes()); // tpq = 96
        
        let track = vec![
            0x00, 0xFF, 0x51, 0x03, 0x07, 0xA1, 0x20, // SetTempo 120BPM
            0x00, 0x90, 0x3C, 0x64,                    // NoteOn C4
            0x60, 0x80, 0x3C, 0x00,                    // NoteOff C4 (delta=96)
            0x00, 0xFF, 0x2F, 0x00,                    // End of Track
        ];
        
        d.extend_from_slice(b"MTrk");
        d.extend_from_slice(&(track.len() as u32).to_be_bytes());
        d.extend_from_slice(&track);
        
        d
    }

    #[test]
    fn test_convert_midi_to_wav() {
        let midi_data = make_dummy_midi();
        let out_path = "test_output.wav";
        
        // テスト実行
        let result = convert_midi_to_wav(&midi_data, out_path, None);
        assert!(result.is_ok(), "変換に失敗しました: {:?}", result.err());

        // 出力されたファイルを確認
        let wav_data = fs::read(out_path).expect("出力WAVファイルが見つかりません");
        assert!(wav_data.len() > 44, "WAVファイルが小さすぎます");
        
        // ヘッダー確認 (RIFF, WAVE)
        assert_eq!(&wav_data[0..4], b"RIFF");
        assert_eq!(&wav_data[8..12], b"WAVE");
        assert_eq!(u16::from_le_bytes([wav_data[22], wav_data[23]]), 2, "WAVはステレオで出力されるべき");

        // クリーンアップ
        let _ = fs::remove_file(out_path);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn test_player_source_reports_stable_format_to_rodio() {
        let midi_data = make_dummy_midi();
        let mut player = Player::new(48_000.0);
        player.load(&midi_data).expect("MIDIのロードに失敗しました");

        let source = PlayerSource::new(player, 48_000);

        assert_eq!(Source::channels(&source), 2, "CLI再生はステレオであるべき");
        assert_eq!(Source::sample_rate(&source), 48_000, "Sourceはデバイスのサンプルレートを使うべき");
        assert_eq!(Source::current_frame_len(&source), None, "rodioにはフォーマット固定の単一ストリームとして見せるべき");
    }

    #[test]
    fn test_cli_chunk_frames_scales_with_sample_rate() {
        assert_eq!(cli_chunk_frames(44_100), 22_050);
        assert_eq!(cli_chunk_frames(48_000), 24_000);
        assert_eq!(cli_chunk_frames(2_000), CLI_MIN_CHUNK_FRAMES);
    }
}
