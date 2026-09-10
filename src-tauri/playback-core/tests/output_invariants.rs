//! Guards the frame-alignment invariant the ring depends on.
//!
//! `pop_into` reports whole frames by dividing the sample count by the channel count.
//! That is only correct while the ring's contents stay a whole number of frames. The
//! invariant holds because capacity is allocated as `frames * channels`, so free space is
//! always frame-aligned and a push can never stop between the channels of one frame. If
//! anyone changes how capacity is computed, these tests fail rather than the audio
//! silently acquiring a permanent channel swap.

use apogee_playback_core::output::{OutputFormat, pcm_ring};

fn stereo() -> OutputFormat {
    OutputFormat {
        sample_rate: 48_000,
        channels: 2,
    }
}

#[test]
fn a_full_ring_never_accepts_a_partial_frame() {
    let (mut tx, mut rx) = pcm_ring(stereo(), 4);

    // Offer far more than fits, in one go.
    let offered: Vec<f32> = (0..100).map(|i| i as f32).collect();
    let written = tx.push_frames(&offered);
    assert_eq!(
        written, 4,
        "should accept exactly the capacity in whole frames"
    );

    // Drain and confirm nothing was torn: every frame is a consecutive pair.
    let mut out = vec![0.0f32; 8];
    let frames = rx.pop_into(&mut out);
    assert_eq!(frames, 4);
    for (i, pair) in out[..8].chunks(2).enumerate() {
        assert_eq!(pair[0], (i * 2) as f32, "frame {i} left channel torn");
        assert_eq!(pair[1], (i * 2 + 1) as f32, "frame {i} right channel torn");
    }
}

#[test]
fn repeated_partial_fills_never_desynchronise_the_channels() {
    let (mut tx, mut rx) = pcm_ring(stereo(), 5);
    let mut next = 0.0f32;

    // Adversarial: push odd-sized frame counts and drain odd-sized reads, many times.
    for round in 0..500 {
        let frames_to_push = (round % 7) + 1;
        let block: Vec<f32> = (0..frames_to_push * 2).map(|i| next + i as f32).collect();
        let written = tx.push_frames(&block);
        next += (written * 2) as f32;

        let frames_to_pop = (round % 5) + 1;
        let mut out = vec![0.0f32; frames_to_pop * 2];
        let got = rx.pop_into(&mut out);

        // Every real frame must be a consecutive (n, n+1) pair. A one-sample slip would
        // pair a right channel with the next frame's left for the rest of the session.
        for pair in out[..got * 2].chunks(2) {
            assert_eq!(
                pair[1] - pair[0],
                1.0,
                "round {round}: channels slipped, got {pair:?}"
            );
            assert_eq!(
                pair[0] as i64 % 2,
                0,
                "round {round}: frame started mid-pair"
            );
        }
    }
}

#[test]
fn mono_and_multichannel_rings_are_also_frame_aligned() {
    for channels in [1u16, 2, 4, 6] {
        let format = OutputFormat {
            sample_rate: 48_000,
            channels,
        };
        let (mut tx, mut rx) = pcm_ring(format, 3);
        let ch = channels as usize;

        let offered: Vec<f32> = (0..ch * 10).map(|i| i as f32).collect();
        let written = tx.push_frames(&offered);
        assert_eq!(
            written, 3,
            "{channels}ch: capacity should be 3 whole frames"
        );

        let mut out = vec![0.0f32; ch * 3];
        assert_eq!(rx.pop_into(&mut out), 3, "{channels}ch");
        for (f, frame) in out.chunks(ch).enumerate() {
            for (c, sample) in frame.iter().enumerate() {
                assert_eq!(
                    *sample,
                    (f * ch + c) as f32,
                    "{channels}ch frame {f} chan {c}"
                );
            }
        }
    }
}
