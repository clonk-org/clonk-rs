# `cargo xtask chaos`

A deterministic regression harness for the question "what does one bad
participant cost everybody else?" in a lockstep session.

```sh
cargo xtask chaos run                      # measure and print
cargo xtask chaos run --profile potato-dialup --ticks 400
cargo xtask chaos record                   # rewrite baseline.json
cargo xtask chaos verify                   # print deltas against the baseline
```

## What it is, and is not

**Report-only.** The baseline policy in `docs/PERFORMANCE.md` requires at least
20 comparable default-branch samples before a timing becomes a blocking
threshold, so `verify` prints deltas and does not fail on them. Two things *do*
fail, because neither is a timing measurement:

- **Determinism** — the same seed run twice must agree exactly. If it does not,
  every other number here is noise. Reproduce it; never retry it.
- **Coverage** — `sometimes`-style assertions (after Antithesis). If no injector
  fired, the suite passed vacuously, and a broken injector looks exactly like a
  clean run.

Everything runs on a virtual clock, so every metric is an exact integer and the
whole report is bit-reproducible. There is no wall-clock measurement anywhere,
which is why this is safe to run on a loaded machine or a shared CI runner.

## Reading the table

`blocked ‰` is the fraction of scheduled ticks on which a **healthy** participant
had to wait — the number that answers the actual question. It is always reported
next to `horizon`, the PreSend lookahead those participants chose, because any
change can drive blocking to zero by spending input latency instead. A one-sided
figure rewards the wrong thing.

`impaired drift` is how far behind the nominal schedule the bad participant
finished; `healthy drift` is the same for everyone else. A slow machine shows up
as drift, not as a missing tick count — it executes every tick, just too slowly.

## Profiles

The link and the machine are separate knobs on purpose. A single blended "lag"
number cannot distinguish "buy a CPU" from "buy an ISP", and the fixes differ, so
`slow-cpu-only` and `bad-link-only` isolate each. The report can therefore be
interpreted by cause instead of treating every delay as a single
network-quality number.

## Coverage boundaries

- **The host's uplink is not shared between peers.** Each participant has its own
  independent simulated link, so adding participants adds no host-side
  contention and `potato-dialup-8p` reports the same numbers as `potato-dialup`.
  Real host uplink is one pipe carrying N copies of every aggregate, and that is
  where a larger game degrades first. Tracked by clonk-org/clonk-rs#1229.
- **Resource transfer is not modelled on the shared sequence space.** The link
  model has competing bulk traffic (`cross_traffic_*_bps`), but the session
  harness does not yet put resource fragments into the same strictly-ordered
  reliable-UDP stream as control. That is the mechanism behind the multi-second
  control freezes seen when a peer is still downloading resources, so the
  numbers here currently understate such a session. Tracked by
  clonk-org/clonk-rs#1230.
- The transport-level view (`clonk_network::sim`) and the session view
  (`clonk_network::sim_session`) are separate models. The session harness does not
  drive real `ReliableUdpEndpointCore` endpoints; it drives the real
  `ControlCoordinator` over a modelled link. Tracked by
  clonk-org/clonk-rs#1231.
