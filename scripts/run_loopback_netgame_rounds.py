#!/usr/bin/env python3
"""Run repeated two-peer netgame rounds on loopback and report each outcome.

This is the reproducible form of the mixed-engine matrix in
clonk-org/clonk-rs#586. `--direction` chooses which side the pinned C++ oracle
plays, so one runner covers port-versus-port (the control), an oracle host with
a port client, and the reverse — each peer with its own binary, install root
and command-line dialect.

It is deliberately opt-in and process-level: it starts real engines, opens real
sockets and plays a real round. Nothing in CI runs it.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import signal
import socket
import subprocess
import sys
import time
import urllib.request
from pathlib import Path
from typing import Callable, Mapping, NamedTuple


#: How each engine announces a failed sync check. Both spellings are always
#: searched, because only the peer that is *not* the control host compares
#: (`C4ControlSyncCheck::Execute`, `C4Control.cpp:469-472`) — so in a mixed
#: round the judge is whichever engine happens to be joining, and matching one
#: engine's wording reports a clean zero for the direction it does not host.
SYNC_LOSS_MESSAGES = ("network desync detected", "Network: Synchronization loss!")


def summarize_round(*, host_log: str, client_log: str) -> dict[str, int]:
    """Reduce one round's two logs to the counts that classify it."""

    both = host_log + client_log
    return {
        "joins": len(re.findall(r"Player join", host_log)),
        "desyncs": sum(both.count(message) for message in SYNC_LOSS_MESSAGES),
    }


def round_is_measurable(summary: dict[str, int]) -> bool:
    """Whether the round admitted both players and is therefore worth reading.

    A host that took the only slot plays on by itself and produces a full,
    healthy-looking log, so every downstream comparison has to gate on this.
    """

    return summary["joins"] >= 2


RUST = "rust"
CPP = "cpp"
ENGINES = (RUST, CPP)


def _peer_prologue(*, engine: str, config: Path, player_name: str) -> list[str]:
    """The part of a peer's argv that differs between the two engines.

    Everything after it — the profile, the scenario and the `/`-parameters —
    is vocabulary both engines share, because the port mirrors
    `C4Game::ParseCommandLine`.

    The oracle reads its config path only from `/config:`
    (`C4Application.cpp:86`) and silently skips any parameter it does not
    recognise (`C4Game.cpp:3141-3292`), so handing it the port's `--config`
    does not fail: it runs on compiled-in defaults, binds the standard ports
    instead of the round's, and never meets its partner. It also needs no
    headless switch, being a `USE_CONSOLE=ON` build, and takes its participant
    from the profile on argv rather than from a name.
    """

    if engine == CPP:
        return [f"/config:{config}"]
    return ["--headless", "--config", str(config), "--player-name", player_name]


class RoundPorts(NamedTuple):
    """Every port one round binds, all resolved before either peer starts."""

    host_tcp: int
    host_udp: int
    reference: int
    client_tcp: int
    client_udp: int


def build_client_command(
    *,
    binary: str | Path,
    config: Path,
    player_name: str,
    profile: Path,
    ports: RoundPorts,
    engine: str = RUST,
) -> list[str]:
    """The joining peer's argv.

    `/join:` takes the host's *reference* port: both engines fetch the
    reference and connect to the addresses it advertises, so the game port is
    the wrong target even though it is the one that carries the session.
    """

    return [
        str(binary),
        *_peer_prologue(engine=engine, config=config, player_name=player_name),
        str(profile),
        f"/join:127.0.0.1:{ports.reference}",
        "/nosignup",
        f"/tcpport:{ports.client_tcp}",
        f"/udpport:{ports.client_udp}",
    ]


def build_host_command(
    *,
    binary: str | Path,
    config: Path,
    player_name: str,
    profile: Path,
    scenario: Path,
    ports: RoundPorts,
    engine: str = RUST,
) -> list[str]:
    """The hosting peer's argv.

    The scenario is passed on argv rather than opened later from the console:
    `/open` starts a local game and silently leaves networking down, so the
    run logs a clean startup and then idles with no lobby and no reference.

    The scenario path must sit inside that peer's own install root. A path
    outside it makes the scenario's folder an install root of its own, and the
    host then loads a different `Material.c4g` than the client does.
    """

    return [
        str(binary),
        *_peer_prologue(engine=engine, config=config, player_name=player_name),
        str(profile),
        str(scenario),
        "/network",
        "/lobby",
        "/nosignup",
        "/console",
        f"/tcpport:{ports.host_tcp}",
        f"/udpport:{ports.host_udp}",
    ]


def launch_peer(
    *,
    command: list[str],
    log: Path,
    environment: Mapping[str, str],
    working_directory: Path,
    opener: Callable[[Path], object] | None = None,
) -> subprocess.Popen:
    """Start one peer with its stdin held open for the life of the round.

    The open pipe is not for sending commands to the client — it never
    receives one. A console engine treats end of file on stdin as a fatal
    read: `CStdApp::ReadStdInCommand` returns false when `read` delivers no
    byte and its caller answers `HR_Failure` (`StdAppUnix.cpp:414-455,
    581-596`). A peer inheriting a closed or `/dev/null` stdin therefore exits
    zero about a second in, having initialised everything and played nothing.
    """

    open_log = opener if opener is not None else _open_log
    return subprocess.Popen(
        command,
        stdin=subprocess.PIPE,
        text=True,
        stdout=open_log(log),
        stderr=subprocess.STDOUT,
        env=dict(environment),
        cwd=str(working_directory),
    )


def _open_log(path: Path):
    path.parent.mkdir(parents=True, exist_ok=True)
    return path.open("w", encoding="utf-8")


def render_process_config(*, name: str, tcp: int, udp: int, reference: int) -> str:
    """One peer's `config.ini`.

    Both peers run on the same machine, so `LocalName`/`Nick` have to be set
    explicitly: left unset they both take the machine name and the host cannot
    tell them apart.
    """

    return (
        "[General]\n"
        f"Name={name}\n"
        'Participants=""\n'
        "ConfigResetSafety=42\n"
        "Language=US\n"
        "LanguageEx=US\n"
        "\n"
        "[Network]\n"
        f"LocalName={name}\n"
        f"Nick={name}\n"
        f"PortTCP={tcp}\n"
        f"PortUDP={udp}\n"
        f"PortRefServer={reference}\n"
        "PortDiscovery=0\n"
        "MasterServerSignUp=false\n"
        "LeagueServerSignUp=false\n"
        "EnableUPnP=false\n"
        "ControlMode=0\n"
        "ControlRate=2\n"
        "AsyncMaxWait=2\n"
        "\n"
        "[Graphics]\n"
        "ResolutionX=640\n"
        "ResolutionY=480\n"
        "Scale=100\n"
        "DisplayMode=1\n"
        "Maximized=false\n"
    )


class HarnessError(RuntimeError):
    """A precondition that would make the round's results meaningless."""


class PeerSetup(NamedTuple):
    """Which engine one peer runs, and where it reads its groups from."""

    binary: Path
    engine: str
    install_root: Path

    @property
    def argv0(self) -> str:
        """The name this peer must be launched under.

        On macOS `main` opens with
        `chdir(dirname(dirname(dirname(dirname(argv[0])))))`
        (`C4WinMain.cpp:231-239`), which walks out of `Clonk.app/Contents/
        MacOS/` to the directory holding the bundle. A console build is not in
        a bundle, so an absolute name walks four real directories out of its
        install root instead — and the working directory the process was given
        is discarded on the way.

        `./clonk` survives it because `dirname(".")` is `"."`: the same chdir
        becomes a no-op and the peer keeps the directory it was started in.
        """

        if self.binary.parent == self.install_root:
            return f"./{self.binary.name}"
        return str(self.binary)


#: The matrix this issue asks for: the control plus each mixed direction,
#: named by which side the oracle takes.
DIRECTIONS: dict[str, tuple[str, str]] = {
    "control": (RUST, RUST),
    "cpp-host": (CPP, RUST),
    "cpp-client": (RUST, CPP),
}


def resolve_peers(
    *,
    direction: str,
    port_binary: Path,
    port_root: Path,
    oracle_binary: Path | None,
) -> tuple[PeerSetup, PeerSetup]:
    """Turn a direction into the two peers that play it.

    A mixed direction with no oracle is refused rather than run on the port:
    every round prints the same summary whichever binaries produced it, so a
    silent fallback would file the control's answer as a mixed-engine one.

    The oracle's install root is the directory holding the executable, which
    is where a `USE_CONSOLE=ON` build looks for its groups — a property of the
    build, not a convention worth a second argument.
    """

    engines = DIRECTIONS.get(direction)
    if engines is None:
        raise HarnessError(f"unknown direction {direction!r}")
    if CPP in engines and oracle_binary is None:
        raise HarnessError(
            f"direction {direction} needs a C++ oracle binary; without one the "
            "round would be the port against itself and read as mixed"
        )

    def peer(engine: str) -> PeerSetup:
        if engine == CPP:
            assert oracle_binary is not None
            return PeerSetup(oracle_binary, CPP, oracle_binary.parent)
        return PeerSetup(port_binary, RUST, port_root)

    return peer(engines[0]), peer(engines[1])


def engine_path(path: Path) -> Path:
    """Absolute and normalised, but with symlinks left standing.

    Both engines derive the install root from the ancestors of the path they
    are handed, and the oracle reaches every one of its groups through a
    symlink into a working checkout. Resolving first would hand the harness a
    path the engine is never given.
    """

    return Path(os.path.abspath(path))


def validate_scenario_inside_install_root(*, scenario: Path, install_root: Path) -> None:
    """Refuse a scenario the engine would resolve against a second root.

    A scenario opened from outside the install root turns its own folder into
    an install root, and the "installed" material group then resolves to the
    overlay the folder chain already contributed rather than to the global
    one. The host renders a partial map behind a `texture not found` warning
    while the client, which resolves its own copy, does not — a content
    difference that presents as a peer disagreement.
    """

    if not scenario.is_relative_to(install_root):
        raise HarnessError(
            f"scenario {scenario} is outside the install root {install_root}; "
            "the peers would load different material groups"
        )


#: The groups both peers must read identically for a round to mean anything.
#: `Graphics.c4g` is deliberately absent — the port ships its own art and the
#: profile does not claim otherwise, so requiring it to match would reject
#: every mixed round over a difference that is not simulated.
SHARED_GROUPS = ("System.c4g", "Material.c4g")

#: Where a group sits relative to an install root. The oracle keeps its groups
#: beside the executable; this repository keeps content under `content/` and
#: its own scripts under `planet/`.
GROUP_SEARCH_PATH = (".", "content", "planet")


def locate_group(install_root: Path, name: str) -> Path | None:
    """The real file a peer rooted here would open for `name`, if any.

    A dangling link answers `None` rather than its target: the engine will not
    stop for a group it cannot open, so the harness has to.
    """

    for directory in GROUP_SEARCH_PATH:
        candidate = install_root / directory / name
        if candidate.exists():
            return candidate.resolve()
    return None


def group_digest(group: Path) -> str:
    """A content identity for one group, packed file or unpacked directory.

    Entry names are folded in relative to the group, so two copies of the same
    submodule commit at different paths agree — which they must, since a
    worktree carries its own `content/` while the oracle's links point at the
    main checkout.
    """

    digest = hashlib.sha256()
    if group.is_file():
        digest.update(group.read_bytes())
        return digest.hexdigest()
    for entry in sorted(group.rglob("*")):
        digest.update(str(entry.relative_to(group)).encode("utf-8"))
        digest.update(b"\0")
        if entry.is_file():
            digest.update(entry.read_bytes())
    return digest.hexdigest()


def validate_shared_content_roots(*, host_root: Path, client_root: Path) -> None:
    """Refuse two roots that would build different worlds from one scenario.

    A mixed round is the first configuration with two install roots, because
    each engine resolves its groups from its own. The oracle's are symlinks
    into a working checkout, which is exactly the kind of link that goes stale
    when a worktree is cleaned up — and neither engine reports the loss.
    """

    for name in SHARED_GROUPS:
        located = {}
        for side, root in (("host", host_root), ("client", client_root)):
            group = locate_group(root, name)
            if group is None:
                raise HarnessError(
                    f"{root} serves no readable {name}; a peer rooted there "
                    "loads nothing where the other loads a group"
                )
            located[side] = group
        if group_digest(located["host"]) != group_digest(located["client"]):
            raise HarnessError(
                f"the peers disagree on {name}: {located['host']} against "
                f"{located['client']}; they would build different worlds from "
                "the same scenario"
            )


def scenario_max_player(scenario: Path) -> int | None:
    """`[Head] MaxPlayer` from an unpacked scenario, if it declares one."""

    head = scenario / "Scenario.txt"
    if not head.is_file():
        return None
    text = head.read_text(encoding="latin-1")
    match = re.search(r"^MaxPlayer=(\d+)", text, re.MULTILINE)
    return int(match.group(1)) if match else None


def require_multiplayer_scenario(scenario: Path) -> None:
    """Refuse a scenario that cannot seat both peers.

    A one-slot scenario admits the host and leaves the joining peer with no
    player at all, which the engine answers by deactivating it after
    `C4NetDeactivationDelay` (`C4Network2Client.cpp:648-654`). That is a
    correct response to a badly chosen scenario, and it is indistinguishable
    from a client that failed to join.
    """

    maximum = scenario_max_player(scenario)
    if maximum is not None and maximum < 2:
        raise HarnessError(
            f"{scenario.name} declares MaxPlayer={maximum}; a two-peer round "
            "there seats only the host and the client is deactivated"
        )


REPOSITORY = Path(__file__).resolve().parents[1]
DEFAULT_BINARY = REPOSITORY / "target" / "play" / "clonk-app"
DEFAULT_SCENARIO = REPOSITORY / "content" / "Melees.c4f" / "Massif.c4s"


def render_player_profile(*, name: str, color: int) -> str:
    """One directory-backed `.c4p` profile body."""

    return (
        "[Player]\n"
        f"Name={name}\n"
        "Comment=loopback netgame round\n"
        "Score=0\nRounds=0\nRoundsWon=0\nRoundsLost=0\nTotalPlayingTime=0\n"
        "\n"
        "[Preferences]\n"
        f"Color={color}\n"
        f"ColorDw={0xFF0000 if color == 0 else 0x0000FF}\n"
        "Position=0\nControl=0\nMouse=0\nAutoStopControl=1\nAutoContextMenu=1\n"
    )


def write_player_profile(path: Path, *, name: str, color: int) -> None:
    path.mkdir(parents=True, exist_ok=True)
    (path / "Player.txt").write_text(
        render_player_profile(name=name, color=color), encoding="ascii"
    )


def allocate_round_ports() -> RoundPorts:
    """Bind-and-release five ports so concurrent runs cannot collide."""

    def free(kind: int) -> int:
        probe = socket.socket(socket.AF_INET6, kind)
        try:
            probe.setsockopt(socket.IPPROTO_IPV6, socket.IPV6_V6ONLY, 0)
            probe.bind(("::", 0))
            if kind == socket.SOCK_STREAM:
                probe.listen(1)
            return probe.getsockname()[1]
        finally:
            probe.close()

    return RoundPorts(
        host_tcp=free(socket.SOCK_STREAM),
        host_udp=free(socket.SOCK_DGRAM),
        reference=free(socket.SOCK_STREAM),
        client_tcp=free(socket.SOCK_STREAM),
        client_udp=free(socket.SOCK_DGRAM),
    )


def reference_is_published(port: int, timeout: float = 1.0) -> bool:
    """Whether the host's reference server answers yet.

    Polling this beats sleeping: it is the same document both engines fetch to
    find each other, so it reports readiness rather than elapsed time.
    """

    try:
        with urllib.request.urlopen(
            f"http://127.0.0.1:{port}", timeout=timeout
        ) as body:
            return b"[Reference]" in body.read()
    except OSError:
        return False


def await_reference(port: int, *, timeout: float) -> bool:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if reference_is_published(port):
            return True
        time.sleep(0.5)
    return False


def run_round(
    root: Path,
    *,
    host_peer: PeerSetup,
    client_peer: PeerSetup,
    scenario: Path,
    join_wait_seconds: float,
    play_seconds: float,
) -> dict[str, object]:
    """Play one round and return its summary.

    The host starts first because the client needs its reference document to
    exist. `/start 0` goes in only after the client has had time to be
    admitted: no readiness signal here beats waiting. A host-side "connected"
    line and a client-side "script engine linked" line both fire while the
    client is still preparing, and starting on either yields fewer complete
    rounds than a fixed wait does.
    """

    ports = allocate_round_ports()
    (root / "host").mkdir(parents=True, exist_ok=True)
    (root / "client").mkdir(parents=True, exist_ok=True)
    (root / "host" / "config.ini").write_text(
        render_process_config(
            name="LoopbackHost",
            tcp=ports.host_tcp,
            udp=ports.host_udp,
            reference=ports.reference,
        ),
        encoding="utf-8",
    )
    (root / "client" / "config.ini").write_text(
        render_process_config(
            name="LoopbackClient",
            tcp=ports.client_tcp,
            udp=ports.client_udp,
            reference=allocate_round_ports().reference,
        ),
        encoding="utf-8",
    )
    write_player_profile(
        root / "profiles" / "HostPlayer.c4p", name="HostPlayer", color=0
    )
    write_player_profile(
        root / "profiles" / "ClientPlayer.c4p", name="ClientPlayer", color=1
    )

    environment = dict(os.environ)
    environment.setdefault("RUST_LOG", "info")
    host = launch_peer(
        command=build_host_command(
            binary=host_peer.argv0,
            config=root / "host" / "config.ini",
            player_name="HostPlayer",
            profile=root / "profiles" / "HostPlayer.c4p",
            scenario=scenario,
            ports=ports,
            engine=host_peer.engine,
        ),
        log=root / "host.log",
        environment=environment,
        working_directory=host_peer.install_root,
    )

    client = None
    try:
        if not await_reference(ports.reference, timeout=90.0):
            return {
                "error": "host never published a reference",
                "joins": 0,
                "desyncs": 0,
                "measurable": False,
            }

        # Two engines cannot hold one install root's update lock at once.
        client_environment = dict(environment)
        client_environment["LC_GAME_UPDATE_RECOVERY_COMPLETE"] = "1"
        client = launch_peer(
            command=build_client_command(
                binary=client_peer.argv0,
                config=root / "client" / "config.ini",
                player_name="ClientPlayer",
                profile=root / "profiles" / "ClientPlayer.c4p",
                ports=ports,
                engine=client_peer.engine,
            ),
            log=root / "client.log",
            environment=client_environment,
            working_directory=client_peer.install_root,
        )

        time.sleep(join_wait_seconds)
        write_console_command(host, "/start 0")
        time.sleep(play_seconds)
        write_console_command(host, "/quit")
        time.sleep(2.0)
    finally:
        stop_peer(client)
        stop_peer(host)

    summary = summarize_round(
        host_log=read_log(root / "host.log"),
        client_log=read_log(root / "client.log"),
    )
    summary["measurable"] = round_is_measurable(summary)
    return summary


def write_console_command(process: subprocess.Popen, command: str) -> None:
    if process.stdin is None or process.poll() is not None:
        return
    try:
        process.stdin.write(f"{command}\n")
        process.stdin.flush()
    except OSError:
        pass


def stop_peer(process: subprocess.Popen | None) -> None:
    if process is None:
        return
    if process.poll() is None:
        process.send_signal(signal.SIGTERM)
        try:
            process.wait(timeout=5.0)
        except subprocess.TimeoutExpired:
            process.kill()
    if process.stdout is not None:
        process.stdout.close()


def read_log(path: Path) -> str:
    return path.read_text("utf-8", "replace") if path.is_file() else ""


def build_argument_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rounds", type=int, default=4)
    parser.add_argument("--binary", type=Path, default=DEFAULT_BINARY)
    parser.add_argument(
        "--oracle",
        type=Path,
        default=None,
        help=(
            "the pinned C++ engine, a USE_CONSOLE=ON build; its install root "
            "is the directory it sits in"
        ),
    )
    parser.add_argument(
        "--direction",
        choices=sorted(DIRECTIONS),
        default="control",
        help="which side, if either, the oracle plays",
    )
    parser.add_argument("--scenario", type=Path, default=DEFAULT_SCENARIO)
    parser.add_argument("--install-root", type=Path, default=REPOSITORY)
    parser.add_argument("--join-wait-seconds", type=float, default=15.0)
    parser.add_argument("--play-seconds", type=float, default=30.0)
    parser.add_argument(
        "--out", type=Path, default=REPOSITORY / "target" / "loopback-netgame"
    )
    return parser


def main(argv: list[str] | None = None) -> int:
    arguments = build_argument_parser().parse_args(argv)
    scenario = engine_path(arguments.scenario)

    try:
        host_peer, client_peer = resolve_peers(
            direction=arguments.direction,
            port_binary=engine_path(arguments.binary),
            port_root=engine_path(arguments.install_root),
            oracle_binary=engine_path(arguments.oracle) if arguments.oracle else None,
        )
        for peer in (host_peer, client_peer):
            if not peer.binary.is_file():
                raise HarnessError(f"missing engine binary: {peer.binary}")
        validate_shared_content_roots(
            host_root=host_peer.install_root,
            client_root=client_peer.install_root,
        )
        # Only the host opens the scenario, so only its root has to hold it.
        validate_scenario_inside_install_root(
            scenario=scenario, install_root=host_peer.install_root
        )
        require_multiplayer_scenario(scenario)
    except HarnessError as error:
        print(str(error), file=sys.stderr)
        return 2

    if arguments.out.exists():
        shutil.rmtree(arguments.out)
    arguments.out.mkdir(parents=True)

    summaries = []
    for index in range(1, arguments.rounds + 1):
        summary = run_round(
            arguments.out / f"round-{index:02d}",
            host_peer=host_peer,
            client_peer=client_peer,
            scenario=scenario,
            join_wait_seconds=arguments.join_wait_seconds,
            play_seconds=arguments.play_seconds,
        )
        summary["round"] = index
        # Recorded per round: a summary read later cannot otherwise tell a
        # mixed result from the control it is supposed to be compared against.
        summary["direction"] = arguments.direction
        summaries.append(summary)
        error = f" error={summary['error']}" if "error" in summary else ""
        print(
            f"round {index}: joins={summary.get('joins', 0)} "
            f"desyncs={summary.get('desyncs', 0)} "
            f"measurable={summary.get('measurable', False)}{error}",
            flush=True,
        )

    (arguments.out / "summary.json").write_text(
        json.dumps(summaries, indent=2), encoding="utf-8"
    )
    measurable = [summary for summary in summaries if summary.get("measurable")]
    desynced = [summary for summary in measurable if summary.get("desyncs", 0)]
    print(
        f"\n{arguments.direction}: {len(measurable)}/{len(summaries)} rounds "
        f"measurable; {len(desynced)} desynced",
        flush=True,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
