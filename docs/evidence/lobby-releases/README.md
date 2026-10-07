# Lobby release diagnostics

clonk-org/clonk-rs#1869 adds optional release information to the lobby and to
synchronization-loss messages. Admission still uses the LegacyClonk engine
compatibility identity; different clonk-rs releases remain playable together.

`RELEASE_DIAGNOSTICS` (capability bit 11) negotiates `PID_PortClientRelease`
(`0x76`). Its payload is a nonnegative little-endian signed 32-bit client ID,
an unsigned byte length, and 1–128 ASCII version bytes (letters, digits, `.`,
`-`, and `+`). The existing capability announcement and voice fields retain
their layout. Older port decoders ignore the new bit and receive no release
report packets. Stock LegacyClonk peers receive neither port announcement nor
release reports.

After JoinData assigns the client ID, the client reports its release when the
host announces support. The host authenticates that ID against the connection,
stores it, and relays it to clients that negotiated diagnostics. Re-announcement
replays the host and existing participants' releases for late joiners. Peer
routes cannot supply authoritative release reports, and the existing forwarding
guard rejects the entire host-only `0x7x` range.

The lobby keeps client identity and release metadata separate. A known mismatch
gets an amber `(!)` marker; its tooltip names the host release and explains that
mixed releases may lose synchronization. Peers without release metadata show
`[?]`, explained as `release unknown` in the tooltip. An unknown host release does not produce a mismatch warning.
On synchronization loss, the message names known clients running a different
release from the host without claiming that the mismatch proves the cause.

The PNGs in this directory use the production software lobby renderer, shipped
fonts and assets, and injected client/release fixtures at 1280×720 and 640×480.
They show a matching host/local client, a different release, and an older peer
with an unknown release. They are rendered fixtures, not a live multiplayer
screen recording. Regenerate them with:

```sh
CLONK_RELEASE_LOBBY_CAPTURE_DIR="$PWD/docs/evidence/lobby-releases" \
  cargo nextest run -p clonk-app \
  -E 'test(lobby_marks_releases_that_differ_from_the_host)'
```
