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
- [ ] Recover output devices and keep expensive work out of output callbacks.
- [ ] Continuously adapt playout to jitter and device clock drift.
- [ ] Expose device/route status, input and output selection, a level meter and
  an explicitly requested local microphone test.
- [ ] Complete automated qualification, required repository gates and landing.

Hardware qualification must distinguish synthetic signal tests from real calls.
The Raspberry Pi 4 synthetic AEC, noise suppression, gain control and Opus probe
measured 2.290 ms mean and 3.163 ms p99 per 20 ms frame. It did not open a
microphone, run alongside the game or qualify a physical acoustic path. Windows,
macOS and Linux device removal, permissions, Bluetooth changes and sleep/resume
still require packaged-application checks on the corresponding hardware.

Media encryption authenticates packets to the negotiated connection keys. The
control connection does not authenticate participant identities, and the host
can decrypt relayed speech. This is not identity-authenticated end-to-end
encryption. A TCP-only game connection cannot carry the UDP voice lane.
