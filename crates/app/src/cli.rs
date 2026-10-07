use anyhow::{Context, Result, bail};
use camera::{
    FrameSource,
    replay::{ImageSequence, MonoVideo},
    synthetic::{Scenario, SyntheticSource},
};
use fly_core::config::Config;
use std::path::PathBuf;

pub const HELP: &str = "Fly Tracker\n\nNo source: dashboard shell, or --headless startup check.\nSources (choose one): --synthetic SCENARIO | --sequence MANIFEST | --video MONO.y4m\nOptions: --headless --config FILE --seed N --frames N --period-us N\n         --calibration FILE --report NEW_FILE --compare REPORT (up to 32)\n         --fixture-safety (label-based replay tests only)\n         --export DIRECTORY (write a PGM sequence before replay)\nOffline fit: --fit-calibration POINTS --save-calibration NEW_FILE --max-calibration-error N\nKeys: 1-5 screens, Space pause, arrows step/rewind when paused, Shift larger step\n      Tab focus, Up/Down select track, +/- speed, m mask, PageUp/Down scroll, ? help, q/Ctrl+C exit\nPhysical output is disabled. Synthetic input uses labelled simulation evidence.\n";
#[derive(Debug, Default)]
pub struct Options {
    pub fit_calibration: Option<PathBuf>,
    pub save_calibration: Option<PathBuf>,
    pub max_calibration_error: f64,
    pub headless: bool,
    pub help: bool,
    pub config: Option<PathBuf>,
    pub synthetic: Option<Scenario>,
    pub sequence: Option<PathBuf>,
    pub video: Option<PathBuf>,
    pub seed: u64,
    pub frames: u64,
    pub period_us: u64,
    pub calibration: Option<PathBuf>,
    pub report: Option<PathBuf>,
    pub comparisons: Vec<PathBuf>,
    pub fixture_safety: bool,
    pub export: Option<PathBuf>,
}
impl Options {
    pub fn parse(arguments: impl IntoIterator<Item = String>) -> Result<Self> {
        let mut options = Self {
            seed: 42,
            frames: 300,
            period_us: 10_000,
            max_calibration_error: 0.01,
            ..Self::default()
        };
        let mut arguments = arguments.into_iter();
        while let Some(argument) = arguments.next() {
            match argument.as_str() {
                "--fit-calibration" => {
                    options.fit_calibration = Some(value(&mut arguments, &argument)?.into())
                }
                "--save-calibration" => {
                    options.save_calibration = Some(value(&mut arguments, &argument)?.into())
                }
                "--max-calibration-error" => {
                    options.max_calibration_error = value(&mut arguments, &argument)?.parse()?
                }
                "--headless" => options.headless = true,
                "--help" | "-h" => options.help = true,
                "--fixture-safety" => options.fixture_safety = true,
                "--config" => options.config = Some(value(&mut arguments, &argument)?.into()),
                "--synthetic" => {
                    let name = value(&mut arguments, &argument)?;
                    options.synthetic = Some(
                        serde_json::from_value(serde_json::Value::String(name))
                            .context("Unknown synthetic scenario; see README")?,
                    );
                }
                "--sequence" => options.sequence = Some(value(&mut arguments, &argument)?.into()),
                "--video" => options.video = Some(value(&mut arguments, &argument)?.into()),
                "--seed" => {
                    options.seed = value(&mut arguments, &argument)?
                        .parse()
                        .context("Invalid seed")?
                }
                "--frames" => {
                    options.frames = value(&mut arguments, &argument)?
                        .parse()
                        .context("Invalid frame count")?
                }
                "--period-us" => {
                    options.period_us = value(&mut arguments, &argument)?
                        .parse()
                        .context("Invalid frame period")?
                }
                "--calibration" => {
                    options.calibration = Some(value(&mut arguments, &argument)?.into())
                }
                "--report" => options.report = Some(value(&mut arguments, &argument)?.into()),
                "--compare" => {
                    anyhow::ensure!(
                        options.comparisons.len() < 32,
                        "At most 32 comparison reports"
                    );
                    options
                        .comparisons
                        .push(value(&mut arguments, &argument)?.into());
                }
                "--export" => options.export = Some(value(&mut arguments, &argument)?.into()),
                _ => bail!("Unknown argument: {argument}. Use --help"),
            }
        }
        let sources = usize::from(options.synthetic.is_some())
            + usize::from(options.sequence.is_some())
            + usize::from(options.video.is_some());
        anyhow::ensure!(sources <= 1, "Select only one frame source");
        anyhow::ensure!(
            options.fit_calibration.is_some() == options.save_calibration.is_some(),
            "Fit requires both --fit-calibration and --save-calibration"
        );
        anyhow::ensure!(
            options.fit_calibration.is_none() || sources == 0,
            "Offline fitting cannot also run a frame source"
        );
        anyhow::ensure!(
            options.frames > 0 && options.period_us > 0,
            "Frame count and period must be positive"
        );
        anyhow::ensure!(
            sources > 0 || options.report.is_none() && options.export.is_none(),
            "Report/export requires a source"
        );
        Ok(options)
    }
    pub fn source(&self, config: &Config) -> Result<Option<(Box<dyn FrameSource>, String)>> {
        if let Some(scenario) = self.synthetic {
            return Ok(Some((
                Box::new(SyntheticSource::new(
                    config.frame_size,
                    scenario,
                    self.seed,
                    self.frames,
                    self.period_us,
                )?),
                format!(
                    "synthetic:{scenario:?}:seed={}:frames={}:period={}",
                    self.seed, self.frames, self.period_us
                ),
            )));
        }
        if let Some(path) = &self.sequence {
            return Ok(Some((
                Box::new(ImageSequence::open(path)?),
                format!("sequence:{}", path.display()),
            )));
        }
        if let Some(path) = &self.video {
            return Ok(Some((
                Box::new(MonoVideo::open(path)?),
                format!("video:{}", path.display()),
            )));
        }
        Ok(None)
    }
}
fn value(arguments: &mut impl Iterator<Item = String>, option: &str) -> Result<String> {
    arguments
        .next()
        .with_context(|| format!("{option} requires a value"))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_conflicts_and_invalid_values_are_rejected() {
        let parse = |args: &[&str]| Options::parse(args.iter().map(|arg| (*arg).to_string()));
        assert!(parse(&["--synthetic", "slow_fly", "--video", "x"]).is_err());
        assert!(parse(&["--frames", "0"]).is_err());
        assert!(parse(&["--seed"]).is_err());
        assert!(parse(&["--unknown"]).is_err());
        assert!(parse(&["--headless", "--synthetic", "slow_fly"]).is_ok());
    }
}
