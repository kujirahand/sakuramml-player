use rustysynth::{SoundFont, Synthesizer, SynthesizerSettings};
use std::fs::File;

fn main() {
    let mut f = File::open("www/fonts/TimGM6mb.sf2").unwrap();
    let sf2 = SoundFont::new(&mut f).unwrap();
    let settings = SynthesizerSettings::new(44100);
    let mut synth = Synthesizer::new(&sf2, &settings).unwrap();
    
    synth.note_on(0, 60, 100);
    let mut left = vec![0.0f32; 128];
    let mut right = vec![0.0f32; 128];
    synth.render(&mut left, &mut right);
    println!("Rendered! max: {}", left.iter().cloned().fold(0.0, f32::max));
}
