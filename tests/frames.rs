//! The frame cap the person set, and the way it reaches the runtime.
//!
//! The setting is only worth having if the runtime really draws by it: a number that changes
//! nothing would be worse than no setting at all. The runtime asks the application with
//! `App::frame_limit`, so that is what these tests ask too.

use std::time::Duration;

use qdesk::app::Desk;
use qdesk::settings::{FRAME_CAP_LOCAL, FRAME_CAP_MOST, FRAME_CAP_REMOTE, Prefs};
use qframe::runtime::{App, FrameLimit};
use qframe::storage::Settings;

/// A desktop with `prefs`, and nothing of the machine it runs on.
fn desk(prefs: Prefs) -> Desk {
    Desk::new(None, Some(0), Box::new(|| 0)).settings(Settings::in_memory(), prefs, Vec::new())
}

#[test]
fn without_a_chosen_cap_the_runtime_draws_sixty_frames_here_and_twenty_over_ssh() {
    let limit = desk(Prefs::default()).frame_limit();
    assert_eq!(limit.frames_per_second(false), Some(u32::from(FRAME_CAP_LOCAL)));
    assert_eq!(limit.frames_per_second(true), Some(u32::from(FRAME_CAP_REMOTE)));
}

#[test]
fn a_chosen_cap_reaches_the_runtime_on_every_connection() {
    let limit = desk(Prefs { frame_cap: Some(10), ..Prefs::default() }).frame_limit();
    assert_eq!(limit.frames_per_second(false), Some(10), "the number the person chose is the one drawn by");
    assert_eq!(limit.frames_per_second(true), Some(10), "a chosen number stands over the link");
    assert_ne!(limit, FrameLimit::default(), "a setting that changes nothing is no setting");
}

#[test]
fn a_cap_outside_the_range_reaches_the_runtime_as_the_range_allows() {
    let most = desk(Prefs { frame_cap: Some(u16::MAX), ..Prefs::default() }).frame_limit();
    assert_eq!(most.frames_per_second(false), Some(u32::from(FRAME_CAP_MOST)));
    let least = desk(Prefs { frame_cap: Some(0), ..Prefs::default() }).frame_limit();
    assert_eq!(least.frames_per_second(false), Some(1), "never nought frames a second, which would draw nothing");
}

#[test]
fn the_cap_the_person_changes_is_the_one_the_next_frame_is_drawn_by() {
    let mut desk = desk(Prefs::default());
    assert_eq!(desk.frame_limit().frames_per_second(false), Some(u32::from(FRAME_CAP_LOCAL)));
    desk = desk.settings(Settings::in_memory(), Prefs { frame_cap: Some(5), ..Prefs::default() }, Vec::new());
    let frames = desk.frame_limit().frames_per_second(false).expect("the desktop always caps its own frames");
    assert_eq!(frames, 5, "the runtime asks before every frame, so the change is the next frame's");
    assert_eq!(Duration::from_secs(1) / frames, Duration::from_millis(200), "a fifth of a second between frames");
}
