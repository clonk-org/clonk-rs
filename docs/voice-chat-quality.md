# Voice chat quality work

This work follows the September 2026 audit. Voice remains presentation-only;
media never enters simulation, lockstep controls or recordings.

- [x] Remove the legacy voice codec, stateless codec interfaces and compatibility
  fallback. Old voice implementations are unsupported. Negotiate the current
  version explicitly and reject older media without interpreting it as controls.
- [x] Use fullband Opus and acoustic echo cancellation that covers long delays.
- [x] Limit voice against the headroom remaining after game sound and music.
- [x] Service media independently of game updates, with fair bounded queues and
  capture/receive timestamps that survive every local handoff.
- [x] Recover microphone devices outside the game and media threads.
- [x] Keep host relaying independent of the host's microphone preference, and
  require a recent authenticated round trip before bypassing the host relay.
- [x] Preserve utterance endings on push-to-talk release and beginnings in voice
  activation, while privacy cancellation immediately discards pending capture.
- [x] Recover output devices and keep expensive work out of output callbacks.
- [x] Continuously adapt playout to jitter and device clock drift.
- [x] Expose device/route status, input and output selection, a level meter and
  an explicitly requested local microphone test.
- [x] Preserve fullband speech through mixer and device sample-rate conversion.
- [x] Reject encrypted replays independently of wrapping media sequence numbers.
- [x] Add deterministic impairment traces, malformed-media checks and a
  repeatable target-hardware CPU probe.

## Setup and privacy

In the normal compatibility profile, open **Voice setup** from Options, the
network lobby or the in-game Options menu. **Ctrl+Shift+V** also opens it from
the menu or a running game. The panel fits a 640×480 window and supports
keyboard, pointer, touch and gamepad input. It offers input/output selection,
activation mode, push-to-talk binding, volume, processing switches and retry.
Selections persist, including a selected device that is temporarily missing.

Opening setup stops live capture and does not open the microphone. **Record**
explicitly records three seconds into bounded memory, closes capture and then
plays it locally. The test works offline with voice disabled and never uses
the network. Closing the panel, changing devices, losing focus or leaving the
session cancels the test. Permission denial, unavailable devices and stalled
playback produce a status instead of silently retrying the recording.

Voice transport capability is negotiated when joining, so enabling voice during
a game works without reconnecting. Microphone capture and listening still
require the user's live opt-in. “UDP voice negotiated” describes capability;
it does not claim a confirmed direct route or a successful end-to-end call.

## Timing and signal quality

Output recovers on a device worker while a separate render worker advances the
mixer. Hardware callbacks consume bounded, timestamped PCM; they discard stale
samples after a playback stall. Voice requests a 256-frame device buffer once
per audio session, with a supported-size clamp and device-default fallback.
Echo cancellation uses actual output PCM and playback timestamps, aligned to
the original microphone capture time, and resets when the output clock or
device changes. Queue depth, underrun callbacks and stale-frame counts are
available for diagnostics.

Opus uses 48 kHz mono, 20 ms frames, a 48 kbit/s target, FEC and DTX. Encoded
payloads are bounded at 512 bytes. Default voice and native device conversion
use prepared 64-tap sinc filters; the explicit linear playback preference is
preserved. A signal test covers the 48→44.1→48 kHz playback path and requires
the 15 kHz component to remain within five percent of its original amplitude.
Capture conversion tests cover 44.1, 48, 96 and 192 kHz and reject alias images.

The jitter target tracks recent arrivals between 40 and 120 ms, including
packets that arrive after their playout deadline. Gradual playback-rate
corrections are limited to one percent. Concealment waits until at most one
frame remains queued, allowing reordered speech to arrive while preserving
playout headroom. A delayed end marker can stop a concealed tail without
replaying audio or reopening the ended stream.

## Automated qualification and limits

Deterministic traces combine 1%, 5% and 10% packet loss, 40–200 ms loss bursts,
reordering, a 40 ms path and up to 60 ms added jitter. They assert bounded queues,
ordered playout, recovery after the route stabilizes and clean-route modeled
playout p95 at or below 150 ms. Separate tests cover eight simulated hours of
clock drift and sequence wrapping. These are not measurements of complete
capture-to-speaker latency or an eight-hour live call.

The manual CPU probe uses production capture conversion, timed 96 kHz echo
reference, AEC, noise suppression, gain control, Opus encoding/decoding and one
channel of native-output conversion. Run it explicitly on the target:

```sh
cargo nextest run --release -p clonk-audio --run-ignored all \
  -E 'test(voice_full_pipeline_cpu_budget)' --success-output immediate
```

The Raspberry Pi 4 probe measured 3.712 ms mean and 4.399 ms p99 per 20 ms frame.
It did not open a microphone, run alongside the game or qualify a physical
acoustic path. The older Pi could not be reached by its supplied hostname.
Packaged Windows, macOS and Linux device removal, permissions, Bluetooth changes,
sleep/resume, real-room listening comparisons and a live-call soak remain
hardware qualification work. Synthetic checks do not establish those outcomes.

## Media protection

V3 is the only supported voice protocol. A checked 64-bit nonce counter is
shared across cipher clones and fails closed on exhaustion. The authenticated
receive path maintains a bounded 2,048-packet replay window, shared across all
speakers on the route. It updates only after successful authentication, accepts
reordering and rejects duplicates even after the 16-bit media sequence wraps.
The design follows the nonce uniqueness requirement in
[RFC 8439 §2.6](https://www.rfc-editor.org/rfc/rfc8439.html#section-2.6) and the
bounded replay-window approach in
[RFC 3711 §3.3.2](https://www.rfc-editor.org/rfc/rfc3711.html#section-3.3.2).
Tests corrupt every byte of an encrypted packet, truncate it at every position,
and exercise oversized input and a deterministic malformed Opus corpus.

Media encryption authenticates packets to the negotiated connection keys. The
control connection does not authenticate participant identities, and the host
can decrypt relayed speech. This is not identity-authenticated end-to-end
encryption. A TCP-only game connection cannot carry the UDP voice lane.
