//! Tries the word hints on WAVs:
//! `cargo run -p stt --release --example hotwords -- MODEL_DIR greedy|beam SCORE a.wav b.wav...`
use stt::Stt;
use stt::sherpa::SherpaParakeet;

fn main() -> anyhow::Result<()> {
    let a: Vec<String> = std::env::args().collect();
    let mut s = SherpaParakeet::new(&a[1], "cpu", 8);
    if a[2] == "beam" {
        s.hints = vec!["Tauri".into(), "StickyBot".into(), "Claude".into()];
        s.hint_score = a[3].parse()?;
    }
    s.load()?;
    for w in &a[4..] {
        let mut r = hound::WavReader::open(w)?;
        let rate = r.spec().sample_rate;
        let x: Vec<f32> = r.samples::<i16>().map(|v| v.unwrap() as f32 / 32768.0).collect();
        let t = std::time::Instant::now();
        let text = s.transcribe(&x, rate)?;
        println!("{w}: {text} ({} ms)", t.elapsed().as_millis());
    }
    Ok(())
}
