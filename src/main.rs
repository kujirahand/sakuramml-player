use std::env;
use std::fs;
use wav_io;
use sakuramml_player::player::Player;
use sakuramml_player::soundfont;

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

    let mut head = wav_io::new_mono_header();
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
        eprintln!("使い方: {} <input.mid> [output.wav]", args[0]);
        std::process::exit(1);
    }

    let input_path = &args[1];
    let output_path = if args.len() >= 3 {
        args[2].clone()
    } else {
        format!("{}.wav", input_path)
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

    println!("WAVファイルに書き出しています: {}", output_path);
    match convert_midi_to_wav(&midi_data, &output_path, sf2_data.as_deref()) {
        Ok(_) => println!("完了しました。WAVファイルを保存しました: {}", output_path),
        Err(e) => {
            eprintln!("エラー: {}", e);
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

        // クリーンアップ
        let _ = fs::remove_file(out_path);
    }
}
