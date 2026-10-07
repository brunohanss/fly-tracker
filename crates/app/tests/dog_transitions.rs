mod support;
use fly_core::LockoutReason;
fn check_sequence(name: &str, dogs: &[bool], expected_issued: u64) -> anyhow::Result<()> {
    support::check_sequence(
        "dog-transition",
        5,
        LockoutReason::Dog,
        name,
        dogs,
        expected_issued,
    )
}
#[test]
fn virtual_aiming_stops_on_first_dog_frame() -> anyhow::Result<()> {
    check_sequence(
        "dog-entry.json",
        &[
            false, false, false, false, false, true, true, true, true, true,
        ],
        3,
    )
}
#[test]
fn virtual_aiming_starts_on_first_clear_frame_after_dog_leaves() -> anyhow::Result<()> {
    check_sequence(
        "dog-exit.json",
        &[
            true, true, true, true, true, false, false, false, false, false,
        ],
        5,
    )
}
#[test]
fn virtual_aiming_stops_and_resumes_without_losing_stationary_tracks() -> anyhow::Result<()> {
    check_sequence(
        "dog-cycle.json",
        &[
            false, false, false, false, false, true, true, true, true, true, false, false, false,
            false, false,
        ],
        8,
    )
}

#[cfg(feature = "nanodet-model-tests")]
#[test]
fn real_nanodet_dog_transition_never_issues_a_command() -> anyhow::Result<()> {
    support::check_model_sequences(
        "dog-transition",
        &["dog-entry.json", "dog-exit.json", "dog-cycle.json"],
        LockoutReason::Dog,
    )
}
