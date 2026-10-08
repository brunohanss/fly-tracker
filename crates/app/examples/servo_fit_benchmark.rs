use aiming::commissioning::ServoSamples;
use std::{io::Write, path::Path, time::Instant};
fn main() -> anyhow::Result<()> {
    let path = std::env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("usage: servo_fit_benchmark SAMPLES.json"))?;
    let samples = ServoSamples::load(Path::new(&path))?;
    for _ in 0..100 {
        std::hint::black_box(samples.fit("simulation".into(), 0.01)?);
    }
    let mut timings = Vec::with_capacity(1000);
    for _ in 0..1000 {
        let start = Instant::now();
        std::hint::black_box(samples.fit("simulation".into(), 0.01)?);
        timings.push(start.elapsed().as_secs_f64() * 1_000_000.0);
    }
    timings.sort_by(f64::total_cmp);
    writeln!(
        std::io::stdout().lock(),
        "Offline fit, 1000 samples; microseconds P50={:.3} P95={:.3} P99={:.3} max={:.3}",
        timings[499],
        timings[949],
        timings[989],
        timings[999]
    )?;
    Ok(())
}
