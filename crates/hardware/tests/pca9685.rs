use aiming::{AimRequest, AimingDevice};
use fly_core::{
    Confidence, FrameId, FrameTimestamp, NormalizedAim, PixelPosition, TargetId, config::Config,
    servo::PanTiltConfig,
};
use hardware::pca9685::{
    DriverState, MockPca9685Aimer, Operation, Pca9685Config, PcaError, PwmTiming,
    mock::{InjectedFault, MockEvent, TRACE_CAPACITY},
};
use safety::{Evidence, EvidenceScope, SafetyAuthority, Verdict};

fn aimer() -> anyhow::Result<MockPca9685Aimer> {
    Ok(MockPca9685Aimer::new(
        Pca9685Config::mock_example(),
        PanTiltConfig::simulation_example(),
        EvidenceScope::ReplayFixture,
    )?)
}
fn observe(gate: &mut SafetyAuthority, time: u64, verdict: Verdict) {
    gate.record_camera(FrameId(time), FrameTimestamp(time));
    gate.observe(Evidence {
        frame: FrameId(time),
        timestamp: FrameTimestamp(time),
        scope: EvidenceScope::ReplayFixture,
        verdict,
    });
}
fn clear(gate: &mut SafetyAuthority, time: u64) -> anyhow::Result<()> {
    observe(gate, time, Verdict::Clear(Confidence::new(1.0)?));
    Ok(())
}
fn request(time: u64, x: f64, y: f64) -> anyhow::Result<AimRequest> {
    Ok(AimRequest {
        frame: FrameId(time),
        target: TargetId(1),
        predicted_pixel: PixelPosition::new(10.0, 10.0)?,
        aim: NormalizedAim::new(x, y)?,
    })
}

#[test]
fn initialization_sets_sleep_prescaler_stop_update_and_waits_before_pwm() -> anyhow::Result<()> {
    let mut device = aimer()?;
    assert_eq!(device.snapshot().state, DriverState::Uninitialized);
    let mut gate = SafetyAuthority::new(&Config::default())?;
    clear(&mut gate, 0)?;
    assert!(matches!(
        device.command(&mut gate, FrameTimestamp(0), request(0, 0.0, 0.0)?),
        Err(PcaError::NotReady)
    ));
    assert!(device.mock().trace().is_empty());
    device.initialize()?;
    assert_eq!(device.snapshot().state, DriverState::ReadyInhibited);
    assert_eq!(device.snapshot().inhibit_confirmed, Some(true));
    let trace: Vec<_> = device.mock().trace().iter().copied().collect();
    assert!(matches!(
        trace[0],
        MockEvent::Inhibit {
            inhibited: true,
            success: true
        }
    ));
    let writes: Vec<_> = trace
        .iter()
        .filter_map(|event| match event {
            MockEvent::Write {
                bytes,
                length,
                success: true,
                ..
            } => Some(&bytes[..*length]),
            _ => None,
        })
        .collect();
    assert_eq!(writes[0], &[0xfd, 0x10]);
    assert_eq!(writes[1], &[0, 0x30]);
    assert_eq!(writes[2], &[1, 4]);
    assert_eq!(writes[3], &[0xfe, 121]);
    assert_eq!(writes[4], &[0, 0x20]);
    assert_eq!(writes[5].len(), 65);
    let wait = trace
        .iter()
        .position(|event| {
            matches!(
                event,
                MockEvent::Wait {
                    microseconds: 500,
                    success: true
                }
            )
        })
        .ok_or_else(|| anyhow::anyhow!("missing wait"))?;
    let pwm = trace
        .iter()
        .position(|event| matches!(event, MockEvent::Write { length: 65, .. }))
        .ok_or_else(|| anyhow::anyhow!("missing PWM initialization"))?;
    assert!(wait < pwm);
    assert_eq!(device.mock().register(0), 0x20);
    assert_eq!(device.mock().register(1), 4);
    assert_eq!(device.mock().register(0xfe), 121);
    assert!(!trace.iter().any(|event| matches!(
        event,
        MockEvent::Inhibit {
            inhibited: false,
            ..
        }
    )));
    for channel in 0..16 {
        assert_eq!(device.mock().register(0x09 + channel * 4), 0x10);
    }
    Ok(())
}
#[test]
fn frequency_quantization_and_pulses_use_actual_clock_with_bounded_error() -> anyhow::Result<()> {
    let timing = PwmTiming::new(25_000_000.0, 50.0)?;
    assert_eq!(timing.prescale(), 121);
    assert!((timing.actual_frequency_hz() - 50.02881659836).abs() < 1e-9);
    for (pulse, count) in [(1000.0, 205), (1500.0, 307), (2000.0, 410)] {
        assert_eq!(timing.pulse_ticks(pulse)?.count(), count);
    }
    for clock in [24_000_000.0, 25_000_000.0, 26_000_000.0] {
        for frequency in [40.0, 50.0, 60.0] {
            let timing = PwmTiming::new(clock, frequency)?;
            for pulse in 100..=3000 {
                let ticks = timing.pulse_ticks(f64::from(pulse))?;
                let restored = f64::from(ticks.count()) * timing.tick_us();
                assert!((restored - f64::from(pulse)).abs() <= timing.tick_us() / 2.0 + 1e-9);
            }
        }
    }
    for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 0.0, -1.0, 1e308] {
        assert!(PwmTiming::new(invalid, 50.0).is_err());
        assert!(PwmTiming::new(25_000_000.0, invalid).is_err());
        assert!(timing.pulse_ticks(invalid).is_err());
    }
    assert!(PwmTiming::new(25_000_000.0, 1.0).is_err());
    assert!(PwmTiming::new(25_000_000.0, 2000.0).is_err());
    assert_eq!(
        PwmTiming::new(25_000_000.0, 25_000_000.0 / (4096.0 * 4.0))?.prescale(),
        3
    );
    assert_eq!(
        PwmTiming::new(25_000_000.0, 25_000_000.0 / (4096.0 * 256.0))?.prescale(),
        255
    );
    assert!(timing.pulse_ticks(timing.tick_us() * 4096.0).is_err());
    assert!(timing.pulse_ticks(timing.tick_us() * 0.49).is_err());
    assert_eq!(timing.pulse_ticks(timing.tick_us() * 4095.0)?.count(), 4095);
    Ok(())
}
#[test]
fn configurable_nonadjacent_axes_write_one_image_and_keep_spares_off() -> anyhow::Result<()> {
    let mut profile = PanTiltConfig::simulation_example();
    profile.pan.channel = 15;
    profile.tilt.channel = 2;
    profile.tilt.reversed = true;
    let mut config = Pca9685Config::mock_example();
    config.address = 0x41;
    let mut device = MockPca9685Aimer::new(config, profile, EvidenceScope::ReplayFixture)?;
    device.initialize()?;
    device.mock_mut().clear_trace();
    let mut gate = SafetyAuthority::new(&Config::default())?;
    clear(&mut gate, 0)?;
    assert!(
        device
            .aim(&mut gate, FrameTimestamp(0), request(0, 1.0, 1.0)?)?
            .issued
    );
    let writes: Vec<_> = device
        .mock()
        .trace()
        .iter()
        .filter(|event| matches!(event, MockEvent::Write { .. }))
        .collect();
    assert_eq!(writes.len(), 1);
    assert!(matches!(
        writes[0],
        MockEvent::Write {
            address: 0x41,
            length: 65,
            accepted_data_bytes: 64,
            success: true,
            ..
        }
    ));
    assert_eq!(device.mock().output_ticks(15), Some(410));
    assert_eq!(device.mock().output_ticks(2), Some(205));
    for channel in 0..16 {
        if channel != 15 && channel != 2 {
            assert_eq!(device.mock().output_ticks(channel), None);
        }
    }
    assert_eq!(device.snapshot().state, DriverState::ActiveMock);
    assert_eq!(device.snapshot().mock_commands, 1);
    assert_eq!(device.snapshot().inhibit_confirmed, Some(false));
    let trace = device.mock().trace();
    assert!(matches!(
        trace.front(),
        Some(MockEvent::Inhibit {
            inhibited: true,
            success: true
        })
    ));
    assert!(matches!(
        trace.back(),
        Some(MockEvent::Inhibit {
            inhibited: false,
            success: true
        })
    ));
    Ok(())
}
#[test]
fn every_initialization_operation_failure_latches_and_requires_explicit_recovery()
-> anyhow::Result<()> {
    let mut reference = aimer()?;
    reference.initialize()?;
    let count = reference.mock().operations();
    for operation in 0..count {
        let mut device = aimer()?;
        device
            .mock_mut()
            .inject_after(operation, InjectedFault::Fail);
        assert!(device.initialize().is_err(), "operation={operation}");
        assert_eq!(
            device.snapshot().state,
            DriverState::Faulted,
            "operation={operation}"
        );
        assert_eq!(device.snapshot().inhibit_confirmed, Some(true));
        let mut gate = SafetyAuthority::new(&Config::default())?;
        clear(&mut gate, 0)?;
        assert!(
            device
                .command(&mut gate, FrameTimestamp(0), request(0, 1.0, 1.0)?)
                .is_err()
        );
        assert_eq!(device.snapshot().mock_commands, 0);
        device.initialize()?;
        assert_eq!(device.snapshot().state, DriverState::ReadyInhibited);
        assert!(device.snapshot().last_pulses.is_none());
    }
    Ok(())
}
#[test]
fn every_command_operation_failure_cancels_without_acknowledging_a_command() -> anyhow::Result<()> {
    let mut reference = aimer()?;
    reference.initialize()?;
    let before = reference.mock().operations();
    let mut gate = SafetyAuthority::new(&Config::default())?;
    clear(&mut gate, 0)?;
    reference.command(&mut gate, FrameTimestamp(0), request(0, 1.0, 1.0)?)?;
    let count = reference.mock().operations() - before;
    for operation in 0..count {
        let mut device = aimer()?;
        device.initialize()?;
        device
            .mock_mut()
            .inject_after(operation, InjectedFault::Fail);
        assert!(
            device
                .command(&mut gate, FrameTimestamp(0), request(0, 1.0, 1.0)?)
                .is_err(),
            "operation={operation}"
        );
        assert_eq!(device.snapshot().state, DriverState::Faulted);
        assert_eq!(device.snapshot().mock_commands, 0);
        assert!(device.mock().inhibited());
        assert!(device.snapshot().last_pulses.is_none());
        assert!(
            device
                .command(&mut gate, FrameTimestamp(0), request(0, 1.0, 1.0)?)
                .is_err()
        );
    }
    Ok(())
}
#[test]
fn partial_burst_at_each_byte_never_leaves_the_mock_output_enabled() -> anyhow::Result<()> {
    for accepted_data_bytes in 0..=64 {
        let mut device = aimer()?;
        device.initialize()?;
        // inhibit, mode readback, prescaler readback, then the PWM burst.
        device.mock_mut().inject_after(
            3,
            InjectedFault::PartialWrite {
                accepted_data_bytes,
            },
        );
        let mut gate = SafetyAuthority::new(&Config::default())?;
        clear(&mut gate, 0)?;
        assert!(
            device
                .command(&mut gate, FrameTimestamp(0), request(0, 1.0, -1.0)?)
                .is_err(),
            "bytes={accepted_data_bytes}"
        );
        assert_eq!(device.snapshot().state, DriverState::Faulted);
        assert!(device.mock().inhibited());
        assert_eq!(device.snapshot().mock_commands, 0);
        for channel in 0..16 {
            assert_eq!(device.mock().output_ticks(channel), None);
        }
        assert!(device.mock().trace().iter().any(|event|matches!(event,MockEvent::Write { accepted_data_bytes:accepted,success:false,.. } if *accepted == accepted_data_bytes)));
    }
    Ok(())
}
#[test]
fn disconnected_i2c_and_failed_inhibit_are_reported_without_claiming_safe_output()
-> anyhow::Result<()> {
    let mut device = aimer()?;
    device.initialize()?;
    let mut gate = SafetyAuthority::new(&Config::default())?;
    clear(&mut gate, 0)?;
    device.command(&mut gate, FrameTimestamp(0), request(0, 0.0, 0.0)?)?;
    device.mock_mut().disconnect(true);
    assert!(
        device
            .command(&mut gate, FrameTimestamp(0), request(0, 1.0, 1.0)?)
            .is_err()
    );
    assert_eq!(device.snapshot().state, DriverState::Faulted);
    assert_eq!(device.snapshot().inhibit_confirmed, Some(true));
    device.mock_mut().disconnect(false);
    device.initialize()?;
    device.command(&mut gate, FrameTimestamp(0), request(0, 0.0, 0.0)?)?;
    device.mock_mut().set_inhibit_available(false);
    assert!(device.try_stop().is_err());
    assert_eq!(device.snapshot().inhibit_confirmed, None);
    assert_eq!(device.snapshot().state, DriverState::Faulted);
    // The secondary full-off write still disables the model's PWM registers.
    assert_eq!(device.mock().output_ticks(0), None);
    assert_eq!(device.mock().register(9), 0x10);
    Ok(())
}
#[test]
fn hazards_faults_scope_and_watchdog_cancel_all_output_without_new_pwm_requests()
-> anyhow::Result<()> {
    for verdict in [
        Verdict::Hazard(camera::Hazard::Human),
        Verdict::Hazard(camera::Hazard::Dog),
        Verdict::Hazard(camera::Hazard::Cat),
        Verdict::Unavailable,
        Verdict::Error,
        Verdict::Timeout,
        Verdict::Stale,
        Verdict::Invalid,
        Verdict::Uncertain,
        Verdict::Clear(Confidence::new(0.5)?),
    ] {
        let mut device = aimer()?;
        device.initialize()?;
        let mut gate = SafetyAuthority::new(&Config::default())?;
        clear(&mut gate, 0)?;
        device.command(&mut gate, FrameTimestamp(0), request(0, 1.0, 1.0)?)?;
        device.mock_mut().clear_trace();
        observe(&mut gate, 10_000, verdict);
        assert!(
            !device
                .command(
                    &mut gate,
                    FrameTimestamp(10_000),
                    request(10_000, -1.0, -1.0)?
                )?
                .issued
        );
        assert_eq!(device.snapshot().mock_commands, 1);
        assert_eq!(device.snapshot().state, DriverState::ReadyInhibited);
        assert!(device.mock().inhibited());
        assert!(device.snapshot().last_pulses.is_none());
        assert!(
            !device
                .mock()
                .trace()
                .iter()
                .any(|event| matches!(event, MockEvent::Write { length: 65, .. }))
        );
        clear(&mut gate, 20_000)?;
        device.watchdog(&mut gate, FrameTimestamp(20_000), FrameId(20_000))?;
        assert!(device.mock().inhibited()); // clear evidence alone never resumes work
    }
    let mut device = aimer()?;
    device.initialize()?;
    let mut gate = SafetyAuthority::new(&Config::default())?;
    clear(&mut gate, 0)?;
    device.command(&mut gate, FrameTimestamp(0), request(0, 1.0, 1.0)?)?;
    device.watchdog(&mut gate, FrameTimestamp(50_001), FrameId(0))?;
    assert!(device.mock().inhibited());
    clear(&mut gate, 100_000)?;
    gate.observe(Evidence {
        frame: FrameId(100_000),
        timestamp: FrameTimestamp(100_000),
        scope: EvidenceScope::Live,
        verdict: Verdict::Clear(Confidence::new(1.0)?),
    });
    assert!(
        !device
            .command(
                &mut gate,
                FrameTimestamp(100_000),
                request(100_000, 1.0, 1.0)?
            )?
            .issued
    );
    clear(&mut gate, 200_000)?;
    gate.shutdown();
    assert!(
        !device
            .command(
                &mut gate,
                FrameTimestamp(200_000),
                request(200_000, 1.0, 1.0)?
            )?
            .issued
    );
    Ok(())
}
#[test]
fn altered_configuration_and_external_clock_fail_readback() -> anyhow::Result<()> {
    for (register, value) in [(0, 0x60), (1, 0x0c), (0xfe, 120)] {
        let mut device = aimer()?;
        device.initialize()?;
        device.mock_mut().corrupt_register(register, value);
        let mut gate = SafetyAuthority::new(&Config::default())?;
        clear(&mut gate, 0)?;
        assert!(
            device
                .command(&mut gate, FrameTimestamp(0), request(0, 1.0, 1.0)?)
                .is_err()
        );
        assert_eq!(
            device.snapshot().fault.map(|f| f.operation),
            Some(Operation::Readback)
        );
        assert!(device.mock().inhibited());
    }
    let mut device = aimer()?;
    device.mock_mut().corrupt_register(0, 0x51);
    assert!(device.initialize().is_err());
    assert_eq!(device.snapshot().state, DriverState::Faulted);
    assert!(device.mock().inhibited());
    Ok(())
}
#[test]
fn bad_profiles_addresses_live_scope_and_unknown_fields_are_rejected() -> anyhow::Result<()> {
    serde_json::from_str::<Pca9685Config>(include_str!("../../../config/pca9685-mock.json"))?
        .validate()?;
    for address in [0, 0x3f, 0x70, 0x78, 0x7f, 0x80] {
        let config = Pca9685Config {
            address,
            ..Pca9685Config::mock_example()
        };
        assert!(config.validate().is_err());
    }
    for address in [0x40, 0x41, 0x6f, 0x71, 0x77] {
        Pca9685Config {
            address,
            ..Pca9685Config::mock_example()
        }
        .validate()?;
    }
    assert!(
        Pca9685Config {
            version: 2,
            ..Pca9685Config::mock_example()
        }
        .validate()
        .is_err()
    );
    for frequency_hz in [0.0, 39.0, 61.0, f64::NAN, f64::INFINITY] {
        assert!(
            Pca9685Config {
                frequency_hz,
                ..Pca9685Config::mock_example()
            }
            .validate()
            .is_err()
        );
    }
    assert!(
        MockPca9685Aimer::new(
            Pca9685Config::mock_example(),
            PanTiltConfig::simulation_example(),
            EvidenceScope::Live
        )
        .is_err()
    );
    let mut profile = PanTiltConfig::simulation_example();
    profile.tilt.channel = 0;
    assert!(
        MockPca9685Aimer::new(
            Pca9685Config::mock_example(),
            profile,
            EvidenceScope::ReplayFixture
        )
        .is_err()
    );
    let mut json = serde_json::to_value(Pca9685Config::mock_example())?;
    json["physical_output"] = serde_json::json!(true);
    assert!(serde_json::from_value::<Pca9685Config>(json).is_err());
    Ok(())
}

#[test]
fn corrupt_pwm_readback_and_slow_transactions_cannot_release_inhibition() -> anyhow::Result<()> {
    let mut device = aimer()?;
    device.initialize()?;
    device
        .mock_mut()
        .inject_after(4, InjectedFault::CorruptRead { byte: 2, value: 0 });
    let mut gate = SafetyAuthority::new(&Config::default())?;
    clear(&mut gate, 0)?;
    assert!(
        device
            .command(&mut gate, FrameTimestamp(0), request(0, 1.0, 1.0)?)
            .is_err()
    );
    assert_eq!(
        device.snapshot().fault.map(|f| f.operation),
        Some(Operation::Readback)
    );
    assert!(device.mock().inhibited());
    let mut slow = aimer()?;
    slow.initialize()?;
    slow.mock_mut().set_transaction_latency_us(12_501);
    let mut gate = SafetyAuthority::new(&Config::default())?;
    clear(&mut gate, 0)?;
    let result = slow.command(&mut gate, FrameTimestamp(0), request(0, 1.0, 1.0)?)?;
    assert!(!result.issued);
    assert_eq!(
        result.suppressed_by,
        Some(fly_core::LockoutReason::StaleCamera)
    );
    assert_eq!(slow.snapshot().mock_commands, 0);
    assert!(slow.mock().inhibited());
    assert!(!slow.mock().trace().iter().any(|event| matches!(
        event,
        MockEvent::Inhibit {
            inhibited: false,
            ..
        }
    )));
    let mut overflow = aimer()?;
    overflow.initialize()?;
    overflow.mock_mut().set_transaction_latency_us(1);
    let mut gate = SafetyAuthority::new(&Config::default())?;
    clear(&mut gate, u64::MAX)?;
    assert!(matches!(
        overflow.command(
            &mut gate,
            FrameTimestamp(u64::MAX),
            request(u64::MAX, 1.0, 1.0)?
        ),
        Err(PcaError::Time(_))
    ));
    assert_eq!(overflow.snapshot().state, DriverState::Faulted);
    assert!(overflow.mock().inhibited());
    let mut clock_overflow = aimer()?;
    clock_overflow.initialize()?;
    clock_overflow
        .mock_mut()
        .set_transaction_latency_us(u64::MAX);
    let mut gate = SafetyAuthority::new(&Config::default())?;
    clear(&mut gate, 0)?;
    assert!(
        clock_overflow
            .command(&mut gate, FrameTimestamp(0), request(0, 1.0, 1.0)?)
            .is_err()
    );
    assert_eq!(clock_overflow.snapshot().state, DriverState::Faulted);
    assert!(clock_overflow.mock().inhibited());
    Ok(())
}
#[test]
fn safety_watchdog_uses_independent_safety_age_and_rejects_wrong_frame() -> anyhow::Result<()> {
    let config = Config {
        max_frame_age_us: 100_000,
        max_safety_age_us: 10_000,
        ..Config::default()
    };
    let mut gate = SafetyAuthority::new(&config)?;
    clear(&mut gate, 0)?;
    let mut device = aimer()?;
    device.initialize()?;
    device.command(&mut gate, FrameTimestamp(0), request(0, 1.0, 1.0)?)?;
    device.watchdog(&mut gate, FrameTimestamp(10_001), FrameId(0))?;
    assert!(device.mock().inhibited());
    assert_eq!(device.snapshot().mock_commands, 1);
    clear(&mut gate, 20_000)?;
    let mut mismatched = request(20_000, 1.0, 1.0)?;
    mismatched.frame = FrameId(1);
    assert!(
        !device
            .command(&mut gate, FrameTimestamp(20_000), mismatched)?
            .issued
    );
    assert!(device.mock().inhibited());
    Ok(())
}
#[test]
fn safety_positive_seeded_sequences_never_activate_the_register_adapter() -> anyhow::Result<()> {
    use camera::{
        Frame, FrameSource,
        synthetic::{Scenario, SyntheticSource},
    };
    use safety::SafetyDetector;
    let config = Config::default();
    for scenario in Scenario::ALL.into_iter().filter(|s| s.hazard().is_some()) {
        for seed in [42, 73] {
            let mut source = SyntheticSource::new(config.frame_size, scenario, seed, 10, 10_000)?;
            let mut frame = Frame::new(config.frame_size);
            let mut detector = safety::FixtureDetector;
            let mut gate = SafetyAuthority::new(&config)?;
            let mut device = aimer()?;
            device.initialize()?;
            device.mock_mut().clear_trace();
            while source.next_into(&mut frame)? {
                gate.record_camera(frame.id, frame.timestamp);
                gate.observe(detector.evaluate(&frame));
                let mut requested = request(frame.timestamp.0, 1.0, -1.0)?;
                requested.frame = frame.id;
                assert!(
                    !device
                        .command(&mut gate, frame.timestamp, requested)?
                        .issued,
                    "{scenario:?} seed={seed}"
                );
                assert!(device.mock().inhibited(), "{scenario:?} seed={seed}");
                assert_eq!(
                    device.snapshot().mock_commands,
                    0,
                    "{scenario:?} seed={seed}"
                );
            }
            assert!(
                !device.mock().trace().iter().any(|event| matches!(
                    event,
                    MockEvent::Write { length: 65, .. }
                        | MockEvent::Inhibit {
                            inhibited: false,
                            ..
                        }
                )),
                "{scenario:?} seed={seed}"
            );
        }
    }
    Ok(())
}
#[test]
fn repeated_commands_keep_trace_bounded_and_stop_preserves_fault_state() -> anyhow::Result<()> {
    let mut device = aimer()?;
    device.initialize()?;
    let mut gate = SafetyAuthority::new(&Config::default())?;
    for time in 0..1000 {
        clear(&mut gate, time)?;
        device.command(&mut gate, FrameTimestamp(time), request(time, 0.0, 0.0)?)?;
        assert!(device.mock().trace().len() <= TRACE_CAPACITY);
    }
    device.stop();
    assert!(device.mock().inhibited());
    assert_eq!(device.snapshot().state, DriverState::ReadyInhibited);
    assert_eq!(device.snapshot().mock_commands, 1000);
    device.mock_mut().disconnect(true);
    assert!(device.try_stop().is_err());
    device.mock_mut().disconnect(false);
    device.stop();
    assert_eq!(device.snapshot().state, DriverState::Faulted);
    device.initialize()?;
    assert_eq!(device.snapshot().state, DriverState::ReadyInhibited);
    Ok(())
}
