use std::env;
use std::fs;
#[cfg(not(target_arch = "wasm32"))]
use std::{
    collections::VecDeque,
    sync::mpsc::{self, Receiver},
    thread,
};
use wav_io;
use sakuramml_player::player::Player;
use sakuramml_player::soundfont;

#[cfg(not(target_arch = "wasm32"))]
use rodio::{
    buffer::SamplesBuffer,
    cpal::traits::{DeviceTrait, HostTrait},
    OutputStream,
    OutputStreamHandle,
    Sink,
    Source,
};

#[cfg(not(target_arch = "wasm32"))]
const STREAM_CHUNK_MILLIS: u32 = 250;

#[cfg(not(target_arch = "wasm32"))]
const STREAM_BUFFERED_CHUNKS: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PlaybackMode {
    Stream,
    RenderAll,
}

#[derive(Debug, PartialEq, Eq)]
struct CliOptions {
    input_path: String,
    output_path: Option<String>,
    playback_mode: PlaybackMode,
}

#[cfg(not(target_arch = "wasm32"))]
struct PlayerSource {
    current_chunk: VecDeque<f32>,
    chunk_rx: Receiver<Option<Vec<f32>>>,
    sample_rate: u32,
    finished: bool,
}

#[cfg(not(target_arch = "wasm32"))]
impl PlayerSource {
    fn new(mut player: Player, sample_rate: u32) -> Self {
        let chunk_frames = stream_chunk_frames(sample_rate);
        let first_chunk = player.render_next(chunk_frames);
        let (tx, rx) = mpsc::sync_channel(STREAM_BUFFERED_CHUNKS);

        thread::spawn(move || {
            loop {
                if player.is_render_done() {
                    let _ = tx.send(None);
                    break;
                }

                let chunk = player.render_next(chunk_frames);
                if chunk.is_empty() {
                    let _ = tx.send(None);
                    break;
                }

                if tx.send(Some(chunk)).is_err() {
                    break;
                }
            }
        });

        Self {
            current_chunk: VecDeque::from(first_chunk),
            chunk_rx: rx,
            sample_rate,
            finished: false,
        }
    }

    fn refill_chunk(&mut self) -> bool {
        if self.finished {
            return false;
        }

        match self.chunk_rx.recv() {
            Ok(Some(chunk)) => {
                self.current_chunk = VecDeque::from(chunk);
                true
            }
            Ok(None) | Err(_) => {
                self.finished = true;
                false
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl Iterator for PlayerSource {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if let Some(sample) = self.current_chunk.pop_front() {
                return Some(sample);
            }

            if !self.refill_chunk() {
                return None;
            }
        }
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

fn load_player(midi_data: &[u8], sf2_data: Option<&[u8]>, sample_rate: f32) -> Result<Player, String> {
    if let Some(data) = sf2_data {
        if let Err(e) = soundfont::load_soundfont_bytes(data) {
            eprintln!("SoundFontの解析に失敗しました: {:?}", e);
        }
    }

    let mut player = Player::new(sample_rate);
    if let Err(e) = player.load(midi_data) {
        return Err(format!("MIDIの解析に失敗しました: {}", e));
    }

    Ok(player)
}

fn render_all_samples(mut player: Player) -> Vec<f32> {
    let total_samples = player.get_total_samples();
    player.render_next(total_samples)
}

#[cfg(not(target_arch = "wasm32"))]
fn stream_chunk_frames(sample_rate: u32) -> usize {
    ((sample_rate as usize) * (STREAM_CHUNK_MILLIS as usize) / 1000).max(1)
}

fn parse_cli_args(args: &[String]) -> Result<CliOptions, String> {
    let mut playback_mode = PlaybackMode::Stream;
    let mut positional = Vec::new();

    for arg in args.iter().skip(1) {
        match arg.as_str() {
            "--render-all" => playback_mode = PlaybackMode::RenderAll,
            _ if arg.starts_with('-') => {
                return Err(format!("不明なオプションです: {}", arg));
            }
            _ => positional.push(arg.clone()),
        }
    }

    match positional.len() {
        0 => Err("入力ファイルを指定してください。".to_string()),
        1 => Ok(CliOptions {
            input_path: positional[0].clone(),
            output_path: None,
            playback_mode,
        }),
        2 => Ok(CliOptions {
            input_path: positional[0].clone(),
            output_path: Some(positional[1].clone()),
            playback_mode,
        }),
        _ => Err("引数が多すぎます。入力ファイルと出力ファイルまで指定できます。".to_string()),
    }
}

fn help_text(program: &str) -> String {
    format!(
        "使い方:\n  ストリーム再生: {program} <input.mid or input.mml>\n  全曲レンダリング再生: {program} --render-all <input.mid or input.mml>\n  WAV書き出し: {program} <input.mid or input.mml> <output.wav>\n\nオプション:\n  --render-all  再生前に曲全体をレンダリングしてから再生します\n  --help, -h    このヘルプを表示します"
    )
}

fn print_help(program: &str) {
    println!("{}", help_text(program));
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
fn play_audio(
    midi_data: &[u8],
    sf2_data: Option<&[u8]>,
    playback_mode: PlaybackMode,
) -> Result<(), String> {
    let (_stream, stream_handle, sample_rate) = open_output_stream()?;
    let sample_rate = sample_rate as f32;
    let player = load_player(midi_data, sf2_data, sample_rate)?;
    
    let sink = Sink::try_new(&stream_handle)
        .map_err(|e| format!("Sinkの作成に失敗しました: {}", e))?;

    match playback_mode {
        PlaybackMode::Stream => {
            println!("♪ ストリーム再生を開始します... ({} Hz)", sample_rate as u32);
            sink.append(PlayerSource::new(player, sample_rate as u32));
        }
        PlaybackMode::RenderAll => {
            println!("♪ 音声を準備しています... ({} Hz)", sample_rate as u32);
            let samples = render_all_samples(player);
            println!("♪ 全曲レンダリング後に再生します... ({} Hz)", sample_rate as u32);
            sink.append(SamplesBuffer::new(2, sample_rate as u32, samples));
        }
    }

    sink.sleep_until_end();
    println!("♪ 再生が完了しました。");

    Ok(())
}

/// MIDI データを WAV ファイルに書き出す
fn convert_midi_to_wav(
    midi_data: &[u8], 
    output_path: &str, 
    sf2_data: Option<&[u8]>
) -> Result<(), String> {
    let sample_rate = 44100.0;
    let player = load_player(midi_data, sf2_data, sample_rate)?;
    let samples = render_all_samples(player);

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
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        print_help(&args[0]);
        return;
    }

    if args.len() < 2 {
        print_help(&args[0]);
        std::process::exit(1);
    }

    let options = match parse_cli_args(&args) {
        Ok(options) => options,
        Err(e) => {
            eprintln!("{}", e);
            print_help(&args[0]);
            std::process::exit(1);
        }
    };

    let input_path = &options.input_path;
    let output_path = options.output_path.clone();

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
        let res = sakuramml_player::compile_mml_bytes(&midi_data);
        midi_data = res.bin();
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
            match play_audio(&midi_data, sf2_data.as_deref(), options.playback_mode) {
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

    #[test]
    fn test_render_all_samples_matches_player_total() {
        let midi_data = make_dummy_midi();
        let player = load_player(&midi_data, None, 44_100.0).expect("MIDIのロードに失敗しました");
        let total_samples = player.get_total_samples();
        let samples = render_all_samples(player);

        assert_eq!(samples.len(), total_samples * 2, "全曲レンダリングはステレオPCM長と一致するべき");
    }

    #[test]
    fn test_parse_cli_args_defaults_to_stream_playback() {
        let args = vec!["app".to_string(), "test.mid".to_string()];
        let options = parse_cli_args(&args).expect("引数解析に失敗しました");

        assert_eq!(options.playback_mode, PlaybackMode::Stream);
        assert_eq!(options.input_path, "test.mid");
        assert_eq!(options.output_path, None);
    }

    #[test]
    fn test_parse_cli_args_supports_render_all_and_output_path() {
        let args = vec![
            "app".to_string(),
            "--render-all".to_string(),
            "test.mid".to_string(),
            "out.wav".to_string(),
        ];
        let options = parse_cli_args(&args).expect("引数解析に失敗しました");

        assert_eq!(options.playback_mode, PlaybackMode::RenderAll);
        assert_eq!(options.input_path, "test.mid");
        assert_eq!(options.output_path, Some("out.wav".to_string()));
    }

    #[test]
    fn test_print_help_mentions_render_all() {
        let help = help_text("app");
        assert!(help.contains("使い方:"));
        assert!(help.contains("--render-all"));
        assert!(help.contains("--help, -h"));
    }
}
