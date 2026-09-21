//! Deterministic media traces, with no wall-clock sleeps or real devices.
use super::*;

#[test]
fn voice_recovers_from_loss_bursts_reordering_and_route_jitter() {
    const FRAMES: usize = 1500;
    const BASE: u16 = u16::MAX - 300;
    let mut encoder = clonk_audio::VoiceEncoder::new().unwrap();
    let packets: Vec<_> = (0..FRAMES)
        .map(|frame| {
            let pcm = std::array::from_fn(|i| {
                let t = (frame * clonk_audio::VOICE_FRAME_SAMPLES + i) as f64 / 48_000.0;
                (8_000.0 * (std::f64::consts::TAU * 230.0 * t).sin()) as i16
            });
            encoder.encode(&pcm).unwrap()
        })
        .collect();
    for loss_percent in [1, 5, 10] {
        for burst_frames in [2, 5, 10] {
            let origin = Instant::now();
            let mut jitter = RemoteVoiceJitterBuffer::default();
            let mut seed = 0x6a09e667_u32;
            let mut events = Vec::new();
            for (frame, packet) in packets.iter().enumerate() {
                seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                let impaired = (100..1100).contains(&frame);
                let burst = [450, 750, 1050]
                    .iter()
                    .any(|start| (*start..*start + burst_frames).contains(&frame));
                if impaired && (seed % 100 < loss_percent || burst) {
                    continue;
                }
                // Alternate stable and variable routes, including delays that
                // reverse neighboring packet arrivals. End on a clean route.
                let variation = if impaired && frame % 300 < 150 {
                    u64::from(seed % 61)
                } else {
                    0
                };
                events.push((frame as u64 * 20 + 40 + variation, frame, *packet));
            }
            events.sort_by_key(|event| (event.0, event.1));
            let mut events = events.into_iter().peekable();
            let mut queued = VecDeque::<(usize, f64, bool)>::new();
            let mut last = None;
            let mut played = 0;
            let mut recovered = 0;
            let mut clean_delays = Vec::new();
            let mut rate = 0;
            for millis in (0..FRAMES as u64 * 20 + 500).step_by(5) {
                let now = origin + Duration::from_millis(millis);
                let mut available = 0.005 * (1.0 + f64::from(rate) / 1_000_000.0);
                while available > 0.0 {
                    let Some((frame, remaining, concealed)) = queued.front_mut() else {
                        break;
                    };
                    if *remaining == 0.02 {
                        let age = millis.saturating_sub(*frame as u64 * 20);
                        assert!(age <= 300, "voice accumulated {age} ms at loss={loss_percent}% burst={burst_frames}");
                        if *frame >= 1300 && !*concealed {
                            clean_delays.push(age);
                        }
                    }
                    let consumed = available.min(*remaining);
                    *remaining -= consumed;
                    available -= consumed;
                    if *remaining < 1e-9 {
                        queued.pop_front();
                    }
                }
                while events.peek().is_some_and(|event| event.0 <= millis) {
                    let (arrival, frame, packet) = events.next().unwrap();
                    let at = origin + Duration::from_millis(arrival);
                    let sequence = BASE.wrapping_add(frame as u16);
                    if !jitter.insert(sequence, at, packet) {
                        jitter.observe_arrival(sequence, at);
                    }
                }
                if millis == FRAMES as u64 * 20 + 40 {
                    assert!(jitter.end(BASE.wrapping_add(FRAMES as u16), now));
                }
                let buffered = queued.iter().map(|frame| frame.1).sum::<f64>();
                if jitter.started {
                    rate = jitter.clock.update(
                        now,
                        Duration::from_secs_f64(buffered + jitter.pending.len() as f64 * 0.02),
                        VOICE_FRAME_DURATION * jitter.target_frames as u32,
                    );
                }
                for frame in jitter.drain_ready_with_headroom(
                    now,
                    8_usize.saturating_sub(queued.len()),
                    queued.len(),
                ) {
                    let position = usize::from(frame.sequence.wrapping_sub(BASE));
                    assert!(
                        last.is_none_or(|last| position > last),
                        "duplicate or reordered playout"
                    );
                    last = Some(position);
                    if !frame.concealed {
                        played += 1;
                        if position >= 1300 {
                            recovered += 1;
                        }
                    }
                    queued.push_back((position, 0.02, frame.concealed));
                }
                assert!(queued.len() <= 8);
                assert!(jitter.pending.len() <= MAX_PENDING_VOICE_FRAMES);
                assert!((MIN_VOICE_JITTER_FRAMES..=MAX_VOICE_JITTER_FRAMES)
                    .contains(&jitter.target_frames));
                assert!(rate.abs() <= 10_000);
            }
            eprintln!("TRACE loss={loss_percent}% burst={burst_frames} played={played} recovered={recovered} concealed={} reordered={} queued={}", jitter.concealed_frames, jitter.reordered_frames, queued.len());
            assert!(
                played > FRAMES * 8 / 10,
                "excessive loss: {played}/{FRAMES}"
            );
            assert!(
                recovered >= 198,
                "clean-route recovery lost {} frames",
                200 - recovered
            );
            assert!(jitter.reordered_frames > 0);
            assert!(jitter.concealed_frames > 0);
            assert!(queued.is_empty());
            clean_delays.sort_unstable();
            assert!(
                clean_delays[clean_delays.len() * 95 / 100] <= 150,
                "clean-route playout exceeded 150 ms"
            );
        }
    }
}

#[test]
fn a_late_authenticated_end_stops_audio_already_replaced_by_concealment() {
    let origin = Instant::now();
    let mut state = VoiceChatState::default();
    let payload =
        clonk_audio::test_encode_voice_frame(&[1_000; clonk_audio::VOICE_FRAME_SAMPLES]).unwrap();
    for sequence in 0..3 {
        let mut frame =
            clonk_network::VoiceFrame::outbound(17, 1, sequence, payload.to_vec()).unwrap();
        frame.client_id = 7;
        assert!(state
            .accept_authorized_remote_frame(
                &frame,
                origin + Duration::from_millis(u64::from(sequence) * 20)
            )
            .is_some());
    }
    assert_eq!(
        state
            .drain_remote_playout(7, 17, origin + Duration::from_millis(40), 8, 0)
            .len(),
        3
    );
    let concealed = state.drain_remote_playout(7, 17, origin + Duration::from_millis(100), 8, 0);
    assert_eq!(concealed.len(), 1);
    assert!(concealed[0].concealed);
    let mut end = clonk_network::VoiceFrame::outbound(17, 1, 3, Vec::new()).unwrap();
    end.client_id = 7;
    assert!(state
        .accept_authorized_remote_frame(&end, origin + Duration::from_millis(110))
        .is_some());
    assert!(state
        .drain_remote_playout(7, 17, origin + Duration::from_millis(200), 8, 0)
        .is_empty());
    assert!(state
        .accept_authorized_remote_frame(&end, origin + Duration::from_millis(210))
        .is_none());
    end.payload = payload.to_vec();
    assert!(state
        .accept_authorized_remote_frame(&end, origin + Duration::from_millis(220))
        .is_none());
}
