mod support;
use fly_core::LockoutReason;

fn check_sequence(name: &str, humans: &[bool], issued: u64) -> anyhow::Result<()> {
    support::check_sequence(
        "human-transition",
        9,
        LockoutReason::Human,
        name,
        humans,
        issued,
    )
}

#[test]
fn virtual_aiming_stops_on_first_human_frame() -> anyhow::Result<()> {
    check_sequence(
        "human-entry.json",
        &[
            false, false, false, false, false, true, true, true, true, true,
        ],
        3,
    )
}

#[test]
fn virtual_aiming_resumes_on_first_clear_frame_after_human_leaves() -> anyhow::Result<()> {
    check_sequence(
        "human-exit.json",
        &[
            true, true, true, true, true, false, false, false, false, false,
        ],
        5,
    )
}

#[test]
fn human_entry_and_exit_keep_all_nine_stationary_tracks() -> anyhow::Result<()> {
    check_sequence(
        "human-cycle.json",
        &[
            false, false, false, false, false, true, true, true, true, true, false, false, false,
            false, false,
        ],
        8,
    )
}

#[test]
fn supplied_images_repeat_identically_five_times_per_phase() -> anyhow::Result<()> {
    use camera::{Frame, FrameSource, replay::ImageSequence};
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/human-transition");
    let mut source = ImageSequence::open(&root.join("human-entry.json"))?;
    let mut frame = Frame::new(source.size());
    let mut phase_pixels = Vec::new();
    let mut index = 0;
    while source.next_into(&mut frame)? {
        if index % 5 == 0 {
            if index == 5 {
                assert_ne!(
                    phase_pixels, frame.pixels,
                    "the human image must differ from the empty room"
                );
            }
            phase_pixels.clone_from(&frame.pixels);
        }
        assert_eq!(frame.pixels, phase_pixels);
        index += 1;
    }
    assert_eq!(index, 10);
    Ok(())
}

#[cfg(feature = "nanodet-model-tests")]
#[test]
fn real_nanodet_human_transition_never_issues_a_command() -> anyhow::Result<()> {
    support::check_model_sequences(
        "human-transition",
        &["human-entry.json", "human-exit.json", "human-cycle.json"],
        LockoutReason::Human,
    )
}
