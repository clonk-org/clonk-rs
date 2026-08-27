# Loopback netgame rounds

`scripts/run_loopback_netgame_rounds.py` plays repeated two-peer network rounds
on loopback and reports, per round, whether both players were admitted and
whether either peer detected a desync.

It exists because the mixed-engine matrix in clonk-org/clonk-rs#586 needs a
**control**: before a divergence between the port and the C++ oracle can be
called cross-engine, the port has to be shown not to diverge from itself over
the same path. That question is cheap to ask and was repeatedly asked by hand,
badly.

```sh
cargo build --profile play -p clonk-app
python3 scripts/run_loopback_netgame_rounds.py --rounds 8
```

Nothing in CI runs it. It starts real engines, binds real sockets and plays a
real round; a round costs roughly a minute.

## What it reports

```
round 1: joins=2 desyncs=0 measurable=True
...
8/8 rounds measurable; 0 desynced
```

`measurable` is the gate that matters. A host that ends up holding the only
player slot plays on by itself and produces a complete, healthy-looking log —
every host-side signal looks the same as in a real round. Only the join count
separates them, so a round with `joins < 2` is reported and then ignored.

## Traps this encodes

Each of these produced a wrong conclusion before it was understood, and each is
now either enforced or impossible to hit through this script.

- **stdin must stay open.** A console engine treats end of file on stdin as a
  fatal read: `CStdApp::ReadStdInCommand` returns false when `read` delivers no
  byte and its caller answers `HR_Failure`
  (`StdAppUnix.cpp:414-455,581-596`). A peer started with stdin at `/dev/null`
  initialises everything, logs a clean startup, exits zero about a second in,
  and never plays. Both peers here keep an open pipe; the client's is never
  written to.
- **The scenario goes on argv.** Opening it later from the console starts a
  *local* game and silently leaves networking down — no lobby, no reference
  server, and a startup log that reads as healthy.
- **The scenario must live inside the install root.** Outside it, the
  scenario's own folder becomes an install root, the "installed"
  `Material.c4g` resolves to the overlay the folder chain already contributed,
  and the global group is never opened. The host renders a partial map; the
  client does not. The script refuses such a path.
- **`MaxPlayer` decides whether a round is possible at all.** `Tutorial01.c4s`
  declares `MaxPlayer=1`, so a two-peer round there seats only the host and the
  engine correctly deactivates the client at `C4NetDeactivationDelay`
  (`C4Network2Client.cpp:648-654`). That is indistinguishable from a client
  that failed to join. The script reads the declared limit first and refuses.
- **`/join:` names the reference port, not the game port.** Both engines fetch
  the reference document and connect to the addresses it advertises.
- **Peers need distinct names.** Unset, both take the machine name and the host
  cannot tell them apart.
- **One install root, one update lock.** The second engine needs
  `LC_GAME_UPDATE_RECOVERY_COMPLETE=1` or it refuses to start.
- **There is no readiness signal better than waiting** before `/start`. A
  host-side "connected" line and a client-side "script engine linked" line both
  fire while the client is still preparing; starting on either yields fewer
  complete rounds than the fixed wait.

## What it does not do

The C++ side is not wired up. Driving the oracle needs a different command line
(`/config:` rather than `--config`, no `--headless`), and adding it without
running it would ship an untested path. The port-versus-port direction is what
this script has actually been used for.
