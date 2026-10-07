mod support;
use fly_core::LockoutReason;

fn check_sequence(name: &str, hands: &[bool], issued: u64) -> anyhow::Result<()> {
    support::check_sequence(
        "hand-transition",
        5,
        LockoutReason::Human,
        name,
        hands,
        issued,
    )
}
#[test]
fn virtual_aiming_stops_on_first_frame_with_an_edge_hand() -> anyhow::Result<()> {
    check_sequence(
        "hand-entry.json",
        &[
            false, false, false, false, false, true, true, true, true, true,
        ],
        3,
    )
}
#[test]
fn virtual_aiming_resumes_after_the_edge_hand_leaves() -> anyhow::Result<()> {
    check_sequence(
        "hand-exit.json",
        &[
            true, true, true, true, true, false, false, false, false, false,
        ],
        5,
    )
}
#[test]
fn edge_hand_entry_and_exit_keep_stationary_tracks() -> anyhow::Result<()> {
    check_sequence(
        "hand-cycle.json",
        &[
            false, false, false, false, false, true, true, true, true, true, false, false, false,
            false, false,
        ],
        8,
    )
}

#[test]
fn every_single_frame_hand_reentry_revokes_the_previous_clear_result() -> anyhow::Result<()> {
    check_sequence(
        "hand-reactivity.json",
        &[
            false, false, false, false, false, true, false, true, false, true,
        ],
        5,
    )
}

#[cfg(feature = "nanodet-model-tests")]
#[test]
fn real_nanodet_recognizes_the_edge_hand_as_human() -> anyhow::Result<()> {
    support::check_model_sequences(
        "hand-transition",
        &[
            "hand-entry.json",
            "hand-exit.json",
            "hand-cycle.json",
            "hand-reactivity.json",
        ],
        LockoutReason::Human,
    )
}
