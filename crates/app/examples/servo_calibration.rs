//! Offline fit command. This program cannot access an I2C bus.
use aiming::commissioning::ServoSamples;
use std::{io::Write, path::Path};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 4 {
        anyhow::bail!(
            "usage: servo_calibration SAMPLES.json REPORT.json MAX_ERROR_PIXELS CREATED_UTC"
        );
    }
    let report = ServoSamples::load(Path::new(&args[0]))?.fit(args[3].clone(), args[2].parse()?)?;
    report.save_new(Path::new(&args[1]))?;
    serde_json::to_writer_pretty(std::io::stdout().lock(), &report.validation_pixels)?;
    writeln!(std::io::stdout().lock())?;
    Ok(())
}
