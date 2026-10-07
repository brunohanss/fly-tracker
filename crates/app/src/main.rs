#![forbid(unsafe_code)]

use aiming::calibration::Calibration;
use anyhow::Context;
use app::{
    cli::{HELP, Options},
    pipeline::ReplayPipeline,
    runner::{run_headless, run_observed},
};
use fly_core::config::Config;
use std::{fs::OpenOptions, io};

fn main() -> anyhow::Result<()> {
    let options = Options::parse(std::env::args().skip(1))?;
    if options.help {
        eprint!("{HELP}");
        return Ok(());
    }
    let mut config = if let Some(path) = &options.config {
        Config::load(path).with_context(|| format!("Cannot read {}", path.display()))?
    } else {
        Config::default()
    };
    config.validate()?;
    if !options.headless {
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open("fly-tracker.log")
            .context("Cannot open dashboard log")?;
        tracing_subscriber::fmt()
            .with_ansi(false)
            .with_writer(log)
            .init();
    } else {
        tracing_subscriber::fmt().with_writer(io::stderr).init();
    }
    if let (Some(samples), Some(path)) = (&options.fit_calibration, &options.save_calibration) {
        let created = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_secs();
        let fit = aiming::fit::CalibrationSamples::load(samples)?
            .fit(format!("unix:{created}"), options.max_calibration_error)?;
        fit.calibration.save_new(path)?;
        tracing::info!(fit_rms=fit.fit_residuals.rms,held_out_rms=fit.validation_residuals.rms,held_out_max=fit.validation_residuals.maximum,path=%path.display(),"Offline calibration fit saved; physical acquisition not performed");
        return Ok(());
    }
    let Some((mut source, mut dataset)) = options.source(&config)? else {
        if options.headless {
            tracing::warn!("No source connected; output disabled");
            return Ok(());
        }
        return tui::run().context("Dashboard failed");
    };
    if options.config.is_none() {
        config.frame_size = source.size();
    }
    if let Some(directory) = &options.export {
        let manifest = camera::export::export_sequence(source.as_mut(), directory)?;
        tracing::info!(path=%manifest.display(),"Sequence exported");
    }
    if options.synthetic.is_none() {
        dataset.push_str(&format!(
            ":content={}",
            camera::export::source_fingerprint(source.as_mut())?
        ));
    }
    let configured = !matches!(
        config.safety_backend,
        fly_core::config::SafetyBackend::Unavailable
    );
    anyhow::ensure!(
        !options.fixture_safety || !configured,
        "--fixture-safety conflicts with the configured safety backend"
    );
    if options.fixture_safety || (options.synthetic.is_some() && !configured) {
        config.safety_backend = fly_core::config::SafetyBackend::Fixture;
    }
    dataset.push_str(&format!(":safety={:?}", config.safety_backend));
    let detector = app::safety_backend::build(&config.safety_backend);
    tracing::info!(backend = %detector.diagnostics(), "Replay safety backend selected");
    let calibration = if let Some(path) = &options.calibration {
        Some(Calibration::load(path)?)
    } else {
        Some(Calibration::virtual_plane(source.size())?)
    };
    let mut pipeline = ReplayPipeline::new(source, detector, config.clone(), dataset, calibration)?;
    let report = if options.headless {
        Some(run_headless(&mut pipeline)?)
    } else {
        let runs = options
            .comparisons
            .iter()
            .map(|path| telemetry::RunMetrics::load(path))
            .collect::<Result<Vec<_>, _>>()?;
        let (publication, receiver) = telemetry::bounded(config.telemetry_capacity)?;
        let (commands, command_receiver) = telemetry::bounded(8)?;
        let worker =
            std::thread::spawn(move || run_observed(pipeline, publication, command_receiver));
        let terminal_result = tui::run_connected(receiver, commands, runs);
        let report = worker
            .join()
            .map_err(|_| anyhow::anyhow!("Replay worker panicked"))??;
        terminal_result.context("Dashboard failed")?;
        report
    };
    if let Some(report) = report {
        tracing::info!(
            frames = report.counters.processed_frames,
            issued = report.counters.issued_aims,
            suppressed = report.counters.suppressed_aims,
            unsafe_commands = report.counters.unsafe_commands,
            "Replay complete; physical output disabled"
        );
        for (stage, distribution) in telemetry::Stage::ALL.into_iter().zip(&report.latency) {
            if let Some(stats) = distribution {
                tracing::info!(
                    ?stage,
                    p50_us = stats.p50,
                    p95_us = stats.p95,
                    p99_us = stats.p99,
                    max_us = stats.maximum,
                    samples = stats.count,
                    total = stats.total_samples,
                    "Latency"
                );
            }
        }
        if let Some(path) = &options.report {
            report.save(path)?;
            tracing::info!(path=%path.display(),"Run report saved");
        }
    } else if options.report.is_some() {
        tracing::warn!("Replay interrupted; no completed report saved");
    }
    Ok(())
}
