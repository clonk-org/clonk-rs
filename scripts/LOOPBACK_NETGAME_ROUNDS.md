# Loopback netgame rounds

`scripts/run_loopback_netgame_rounds.py` plays repeated two-peer network rounds
on loopback and reports, per round, whether both players were admitted and
whether either peer detected a desync.

It exists for the mixed-engine matrix in clonk-org/clonk-rs#586, and runs all
three of its directions. The **control** — the port against itself — is what
makes the other two readable: before a divergence can be called cross-engine,
the port has to be shown not to diverge from itself over the same path.

```sh
cargo build --profile play -p clonk-app
python3 scripts/run_loopback_netgame_rounds.py --rounds 8
```

## Directions

`--direction` chooses which side, if either, the oracle plays. Anything but
`control` needs `--oracle`, and is refused without one rather than quietly
falling back to the port — every round prints the same summary whichever
binaries produced it, so a silent fallback would file the control's answer as a
mixed-engine one.

| `--direction` | host | client |
| --- | --- | --- |
| `control` (default) | port | port |
| `cpp-host` | oracle | port |
| `cpp-client` | port | oracle |

```sh
python3 scripts/run_loopback_netgame_rounds.py --rounds 6 \
  --direction cpp-host --oracle "$ORACLE/clonk" \
  --scenario "$ORACLE/Melees.c4f/Massif.c4s"
```

Only the host opens the scenario, so it is the *host's* root the scenario has
to sit inside — which changes with the direction. The oracle's install root is
the directory its executable sits in, and is derived rather than asked for.

Building that oracle is on clonk-org/clonk-rs#586: a `USE_CONSOLE=ON`,
`USE_RUST_ENGINE_VALIDATION=OFF` build of the pinned commit, with the content
groups symlinked in beside the executable.

Nothing in CI runs it. It starts real engines, binds real sockets and plays a
real round; a round costs roughly a minute.

## What it reports

```
round 1: joins=2 desyncs=0 measurable=True
...
control: 8/8 rounds measurable; 0 desynced
```

The direction is printed with the totals and recorded on every round in
`summary.json`, because a summary read later cannot otherwise be told from the
control it is meant to be compared against.

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
- **One install root, one update lock.** Two port peers share a root in the
  control direction, and the second needs `LC_GAME_UPDATE_RECOVERY_COMPLETE=1`
  or it refuses to start.
- **There is no readiness signal better than waiting** before `/start`. A
  host-side "connected" line and a client-side "script engine linked" line both
  fire while the client is still preparing; starting on either yields fewer
  complete rounds than the fixed wait.

## Traps specific to the oracle

- **The oracle must be named relative to its own directory**, and the working
  directory you start it in does not decide the matter. On macOS `main` opens
  with `chdir(dirname(dirname(dirname(dirname(argv[0])))))`
  (`C4WinMain.cpp:231-239`), climbing out of `Clonk.app/Contents/MacOS/` to
  the directory holding the bundle. A console build is not in a bundle, so
  `/…/legacyclonk-oracle-pin/build-console/clonk` walks four real directories
  out of the install root and lands in `/Users/…/Documents/code`. `./clonk`
  survives it — `dirname(".")` is `"."`, so the chdir is a no-op.

  The wrong form produces one line — `Error opening system group file
  (System.c4g)!` — and then a run that never opens a lobby, which from outside
  is indistinguishable from a host that failed to bring networking up. The
  script derives the name from the install root; nothing else about the
  invocation differs.
- **Only the joining peer judges the sync check, and it announces it in its own
  words.** `C4ControlSyncCheck::Execute` returns immediately on the control
  host (`C4Control.cpp:469-472`), so in a mixed round the engine that can see
  a divergence is whichever one is joining — and the oracle says `Network:
  Synchronization loss!` (`C4Control.cpp:500`) where the port says `network
  desync detected`. Counting one wording reports a clean zero for the very
  direction the port cannot see, which is the worst way for this script to be
  wrong: it looks like evidence. Both spellings are counted.
- **The oracle skips parameters it does not recognise.** It reads its config
  path only from `/config:` (`C4Application.cpp:86`), and
  `C4Game::ParseCommandLine` ignores anything it cannot parse
  (`C4Game.cpp:3141-3292`). Handing it the port's `--config` is therefore not
  an error: it runs on compiled-in defaults, binds the standard ports rather
  than the round's, and never meets its partner.
- **A hand-authored `config.ini` is honoured.** Both engines accept the
  unquoted form this script writes, and the oracle rewrites the file in its own
  format on load. Seeding a config by running the engine first is unnecessary.
- **The two roots must serve the same groups.** Each engine resolves `System`
  and `Material` from its own root, and the oracle reaches both through
  symlinks into a working checkout — links that have pointed into a deleted
  worktree before. Neither engine reports the loss. The script compares the
  content of both roots' groups, not their paths, so a worktree with its own
  `content/` is accepted while a real difference is not.

## What it does not do

Nothing here compares simulated state between the peers beyond the desync
detector each engine already runs. Reading a divergence — which frame, which
object, which field — still needs the tracing described on
clonk-org/clonk-rs#586.
