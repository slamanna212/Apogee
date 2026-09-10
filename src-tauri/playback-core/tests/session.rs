//! Adversarial timing around session identity. These are the races the plan calls out.

use apogee_playback_core::session::{
    BufferingReason, CONNECT_BUDGET_MS, Controller, ErrorClass, MAX_CONNECT_ATTEMPTS,
    MAX_RETRY_DELAY_MS, Next, PlaybackState, RETRY_DELAY_MS, STABLE_PLAY_RESET_MS, retry_delay_ms,
};

#[test]
fn switching_station_invalidates_the_previous_session() {
    let mut c = Controller::new();
    let a = c.play("station-a");
    let b = c.play("station-b");
    assert_ne!(a, b);

    // A's connection finally succeeds, far too late.
    assert!(
        !c.on_audible(a, 0),
        "a stale session must not be able to start audio"
    );
    assert_eq!(
        c.state(),
        PlaybackState::Connecting,
        "state must still belong to B"
    );
    assert!(c.on_audible(b, 0));
    assert_eq!(c.snapshot().station_id.as_deref(), Some("station-b"));
}

#[test]
fn selecting_the_same_station_twice_creates_distinct_sessions() {
    let mut c = Controller::new();
    let first = c.play("station-a");
    let second = c.play("station-a");
    assert_ne!(
        first, second,
        "channel identity alone cannot distinguish sessions"
    );
    assert!(
        !c.on_audible(first, 0),
        "the abandoned attempt must not win"
    );
    assert!(c.on_audible(second, 0));
}

#[test]
fn stop_invalidates_work_in_flight_at_every_stage() {
    for stage in ["connect", "buffer", "retry"] {
        let mut c = Controller::new();
        let g = c.play("station-a");
        match stage {
            "buffer" => {
                c.on_buffering(g, BufferingReason::FillingBuffer);
            }
            "retry" => {
                c.on_error(g, ErrorClass::Transient, "reset", 0);
            }
            _ => {}
        }
        c.stop();
        assert_eq!(c.state(), PlaybackState::Stopped, "stage {stage}");
        assert!(
            !c.on_audible(g, 0),
            "stage {stage}: stopped session must stay silent"
        );
        assert!(
            !c.on_buffering(g, BufferingReason::FillingBuffer),
            "stage {stage}"
        );
        assert_eq!(c.state(), PlaybackState::Stopped, "stage {stage}");
    }
}

#[test]
fn a_stopped_controller_accepts_nothing_even_for_the_current_generation() {
    let mut c = Controller::new();
    c.play("station-a");
    let g = c.stop();
    assert!(!c.accepts(g), "nothing may resume a stopped session");
}

#[test]
fn revision_increases_monotonically_so_a_late_snapshot_cannot_win() {
    let mut c = Controller::new();
    let g = c.play("station-a");
    let early = c.snapshot();
    c.on_buffering(g, BufferingReason::FillingBuffer);
    c.on_audible(g, 0);
    let late = c.snapshot();

    assert!(late.revision > early.revision);
    // A consumer ordering by revision discards the stale one.
    assert!(
        early.revision < late.revision,
        "older snapshot must be identifiable as older"
    );
}

#[test]
fn every_observable_change_advances_the_revision() {
    let mut c = Controller::new();
    let g = c.play("s");
    let mut last = c.snapshot().revision;
    for step in 0..4 {
        match step {
            0 => {
                c.on_buffering(g, BufferingReason::FillingBuffer);
            }
            1 => {
                c.on_audible(g, 0);
            }
            2 => {
                c.set_bitrate(g, Some(258));
            }
            _ => {
                c.set_format(g, 44_100);
            }
        }
        let now = c.snapshot().revision;
        assert!(now > last, "step {step} did not advance the revision");
        last = now;
    }
}

#[test]
fn transient_failures_retry_a_bounded_number_of_times() {
    let mut c = Controller::new();
    let g = c.play("station-a");

    // Slow failures: each attempt consumes real time, so the attempt count and the
    // wall-clock budget run out at roughly the same point.
    let mut retries = 0;
    let mut now = 0u64;
    loop {
        match c.on_error(g, ErrorClass::Transient, "timed out", now) {
            Some(Next::Retry { delay_ms, .. }) => {
                retries += 1;
                now += delay_ms + 30_000; // a long, stalling attempt
            }
            Some(Next::GiveUp) => break,
            None => panic!("controller stopped accepting its own generation"),
        }
        assert!(retries < 20, "retries are not bounded");
    }
    assert!(
        retries as u32 >= MAX_CONNECT_ATTEMPTS - 1,
        "gave up after only {retries} retries"
    );
    assert_eq!(c.state(), PlaybackState::Failed);
    assert!(
        c.snapshot().error.is_some(),
        "a final failure must carry a message"
    );
}

#[test]
fn permanent_failures_do_not_retry_at_all() {
    let mut c = Controller::new();
    let g = c.play("station-a");
    let next = c.on_error(g, ErrorClass::Permanent, "invalid credentials", 0);
    assert_eq!(
        next,
        Some(Next::GiveUp),
        "a permanent error must not be retried"
    );
    assert_eq!(c.state(), PlaybackState::Failed);
    assert_eq!(
        c.attempt(),
        0,
        "a permanent error should not consume the retry budget"
    );
}

#[test]
fn retry_attempts_alternate_the_url_extension() {
    assert_eq!(Controller::extension_for_attempt(0), ".ts");
    assert_eq!(Controller::extension_for_attempt(1), ".m3u8");
    assert_eq!(Controller::extension_for_attempt(2), ".ts");
    assert_eq!(Controller::extension_for_attempt(3), ".m3u8");
}

#[test]
fn the_retry_budget_refills_only_after_sustained_playback() {
    let mut c = Controller::new();
    let g = c.play("station-a");

    // Burn a retry, then play briefly and fail again. A short success must not refill.
    c.on_error(g, ErrorClass::Transient, "reset", 0);
    assert_eq!(c.attempt(), 1);
    c.on_audible(g, 1_000);
    c.on_error(g, ErrorClass::Transient, "reset", 2_000);
    assert_eq!(c.attempt(), 2, "a brief success must not refill the budget");

    // Now play long enough to count as stable.
    c.on_audible(g, 10_000);
    c.on_error(
        g,
        ErrorClass::Transient,
        "reset",
        10_000 + STABLE_PLAY_RESET_MS,
    );
    assert_eq!(
        c.attempt(),
        1,
        "sustained playback should have refilled the budget"
    );
}

#[test]
fn a_flapping_stream_still_terminates() {
    // Connect, play briefly, drop; repeatedly. Must not retry forever. Brief playback does
    // not refill the budget, so both limits eventually run out.
    let mut c = Controller::new();
    let g = c.play("station-a");
    let mut now = 0u64;
    for _ in 0..200 {
        c.on_audible(g, now);
        now += 1_000;
        match c.on_error(g, ErrorClass::Transient, "dropped", now) {
            Some(Next::GiveUp) => return,
            Some(Next::Retry { delay_ms, .. }) => now += delay_ms,
            None => panic!("unexpected rejection"),
        }
    }
    panic!("a stream that keeps dropping after brief playback retried forever");
}

#[test]
fn states_project_onto_the_existing_frontend_contract() {
    // src/types/player.ts: 'idle' | 'loading' | 'playing' | 'stopped' | 'error'
    assert_eq!(PlaybackState::Stopped.as_frontend_status(), "stopped");
    assert_eq!(PlaybackState::Connecting.as_frontend_status(), "loading");
    assert_eq!(PlaybackState::Buffering.as_frontend_status(), "loading");
    assert_eq!(PlaybackState::Recovering.as_frontend_status(), "loading");
    assert_eq!(PlaybackState::Playing.as_frontend_status(), "playing");
    assert_eq!(PlaybackState::Failed.as_frontend_status(), "error");

    assert!(PlaybackState::Buffering.is_buffering());
    assert!(PlaybackState::Recovering.is_buffering());
    assert!(!PlaybackState::Playing.is_buffering());
}

#[test]
fn only_real_playback_counts_as_audible_for_scrobbling() {
    // Scrobbling and presence must not credit a connection that never made sound.
    assert!(!PlaybackState::Connecting.is_audible());
    assert!(!PlaybackState::Buffering.is_audible());
    assert!(!PlaybackState::Recovering.is_audible());
    assert!(!PlaybackState::Failed.is_audible());
    assert!(PlaybackState::Playing.is_audible());
}

#[test]
fn a_new_session_clears_the_previous_stations_metadata() {
    let mut c = Controller::new();
    let a = c.play("station-a");
    c.on_audible(a, 0);
    c.set_bitrate(a, Some(258));
    c.set_format(a, 44_100);

    c.play("station-b");
    let snap = c.snapshot();
    assert_eq!(
        snap.bitrate_kbps, None,
        "stale bitrate must not survive a station change"
    );
    assert_eq!(
        snap.sample_rate, None,
        "stale format must not survive a station change"
    );
    assert_eq!(snap.attempt, 0);
}

#[test]
fn the_selected_device_survives_a_station_change() {
    let mut c = Controller::new();
    c.set_device(Some("USB Audio".into()));
    c.play("station-a");
    c.play("station-b");
    c.stop();
    assert_eq!(
        c.snapshot().device.as_deref(),
        Some("USB Audio"),
        "device preference is not per-session"
    );
}

#[test]
fn an_instantly_failing_endpoint_still_gets_the_full_wall_clock_budget() {
    // The regression this guards. A 503 returned while a backend is still starting a stream
    // fails in milliseconds. Counting attempts alone burned all four inside about five
    // seconds and gave up long before the stream could have come up. Measured against a
    // real backend: four attempts spanned 14:36:28 to 14:36:33.
    let mut c = Controller::new();
    let g = c.play("station-a");

    let mut now = 0u64;
    let mut retries = 0;
    loop {
        match c.on_error(g, ErrorClass::Transient, "HTTP 503", now) {
            Some(Next::Retry { delay_ms, .. }) => {
                retries += 1;
                now += delay_ms; // the request itself costs no measurable time
            }
            Some(Next::GiveUp) => break,
            None => panic!("unexpected rejection"),
        }
        assert!(retries < 200, "must still terminate");
    }
    assert!(
        now >= CONNECT_BUDGET_MS,
        "gave up after only {now}ms; the budget is {CONNECT_BUDGET_MS}ms"
    );
    assert!(
        retries as u32 > MAX_CONNECT_ATTEMPTS,
        "only {retries} retries when every failure was free"
    );
}

#[test]
fn backoff_grows_and_is_capped() {
    assert_eq!(retry_delay_ms(1), RETRY_DELAY_MS);
    assert!(retry_delay_ms(2) > retry_delay_ms(1), "delay should grow");
    assert!(retry_delay_ms(3) > retry_delay_ms(2));
    for attempt in 1..50 {
        let delay = retry_delay_ms(attempt);
        assert!(
            delay >= RETRY_DELAY_MS,
            "attempt {attempt} delay {delay} below the floor"
        );
        assert!(
            delay <= MAX_RETRY_DELAY_MS,
            "attempt {attempt} delay {delay} above the cap"
        );
    }
}

#[test]
fn a_fast_failing_endpoint_is_not_hammered() {
    // Backoff has to keep the request count inside the budget modest, or restoring the
    // wall-clock window would just turn one bug into a different one.
    let mut c = Controller::new();
    let g = c.play("station-a");
    let mut now = 0u64;
    let mut requests = 0;
    while let Some(Next::Retry { delay_ms, .. }) =
        c.on_error(g, ErrorClass::Transient, "HTTP 503", now)
    {
        requests += 1;
        now += delay_ms;
        if requests > 500 {
            break;
        }
    }
    assert!(
        requests < 40,
        "{requests} requests inside {CONNECT_BUDGET_MS}ms is hammering the server"
    );
}
