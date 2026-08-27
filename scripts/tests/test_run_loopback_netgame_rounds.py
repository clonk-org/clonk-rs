import importlib.util
import subprocess
import tempfile
import unittest
from unittest import mock
from pathlib import Path


SCRIPT = Path(__file__).resolve().parents[1] / "run_loopback_netgame_rounds.py"
SPEC = importlib.util.spec_from_file_location("loopback_netgame_rounds", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class RoundSummaryTests(unittest.TestCase):
    def test_a_desync_is_counted_from_whichever_peer_logged_it(self):
        """Only the peer that loses the comparison logs it.

        `GameApp::handle_desync` runs on the peer whose digest differs from the
        host's, so a client-side round can end with the host log silent.
        """

        summary = MODULE.summarize_round(
            host_log="Player join: HostPlayer\nPlayer join: ClientPlayer\n",
            client_log="ERROR network desync detected frame=100\n",
        )

        self.assertEqual(summary["desyncs"], 1)

    def test_the_oracles_own_wording_for_a_sync_loss_is_counted(self):
        """Only the non-host compares, so the oracle is sometimes the only judge.

        `C4ControlSyncCheck::Execute` returns immediately on the control host
        (`C4Control.cpp:469-472`) and the peer that does compare announces
        `Network: Synchronization loss!` (`C4Control.cpp:500`). Matching only
        the port's wording therefore reports a clean zero for every direction
        the oracle joins in — the one direction where the port cannot see the
        divergence at all.
        """

        summary = MODULE.summarize_round(
            host_log="Player join: HostPlayer\nPlayer join: ClientPlayer\n",
            client_log=(
                "[info] Network: Synchronization loss!\n"
                "[info] Network: Client Frm 100 Ctrl 50 Rnc 4495 Rn3 0 Cpx 253200 "
                "PXS 0 MMi 0 Obc 700 Oei 705 Sct 1240\n"
                "[info] Network: Host Frm 100 Ctrl 50 Rnc 4495 Rn3 0 Cpx 253200 "
                "PXS 0 MMi 0 Obc 700 Oei 705 Sct 1238\n"
            ),
        )

        self.assertEqual(summary["desyncs"], 1)

    def test_a_round_only_the_host_joined_is_not_a_valid_measurement(self):
        """A solo host still simulates, logs and looks healthy.

        `Tutorial01.c4s` has `MaxPlayer=1`, so a two-peer round there admits
        only the host and C++ deactivates the client at
        `C4NetDeactivationDelay` (`C4Network2Client.cpp:648-654`). Nothing
        about the host's own output distinguishes that from a real round, so
        the join count is the gate.
        """

        solo = MODULE.summarize_round(
            host_log="Player join: HostPlayer\n",
            client_log="",
        )
        both = MODULE.summarize_round(
            host_log="Player join: HostPlayer\nPlayer join: ClientPlayer\n",
            client_log="",
        )

        self.assertEqual(solo["joins"], 1)
        self.assertFalse(MODULE.round_is_measurable(solo))
        self.assertEqual(both["joins"], 2)
        self.assertTrue(MODULE.round_is_measurable(both))


class CommandTests(unittest.TestCase):
    def test_the_client_dials_the_hosts_reference_port_not_its_game_port(self):
        """`/join:` names the reference server, which then hands out the rest.

        Both engines query the reference and connect to the addresses it
        advertises, so pointing `/join:` at the TCP game port reaches a
        listener that never answers a reference query.
        """

        ports = MODULE.RoundPorts(
            host_tcp=21500,
            host_udp=21501,
            reference=21502,
            client_tcp=21510,
            client_udp=21511,
        )
        command = MODULE.build_client_command(
            binary=Path("/engine/clonk-app"),
            config=Path("/run/client/config.ini"),
            player_name="ClientPlayer",
            profile=Path("/run/profiles/ClientPlayer.c4p"),
            ports=ports,
        )

        self.assertIn("/join:127.0.0.1:21502", command)
        self.assertNotIn("/join:127.0.0.1:21500", command)
        self.assertIn("/tcpport:21510", command)
        self.assertIn("/udpport:21511", command)

    def test_the_host_opens_the_scenario_on_argv_with_a_console_lobby(self):
        """`/open` on stdin starts a *local* game and never brings up networking.

        The scenario has to arrive on argv, alongside `/network` and `/lobby`,
        or the run reaches neither a lobby nor a reference server while still
        logging a healthy startup.
        """

        ports = MODULE.RoundPorts(
            host_tcp=21500,
            host_udp=21501,
            reference=21502,
            client_tcp=21510,
            client_udp=21511,
        )
        command = MODULE.build_host_command(
            binary=Path("/engine/clonk-app"),
            config=Path("/run/host/config.ini"),
            player_name="HostPlayer",
            profile=Path("/run/profiles/HostPlayer.c4p"),
            scenario=Path("/install/content/Melees.c4f/Massif.c4s"),
            ports=ports,
        )

        self.assertIn("/install/content/Melees.c4f/Massif.c4s", command)
        for token in ("/network", "/lobby", "/console", "/nosignup"):
            self.assertIn(token, command)
        self.assertIn("/tcpport:21500", command)
        self.assertNotIn("/join:127.0.0.1:21502", command)

    def test_the_oracle_client_joins_with_the_same_engine_parameters(self):
        """The joining direction differs only in the prologue, not the join.

        `/join:`, `/tcpport:` and `/udpport:` are read by the same
        `C4Game::ParseCommandLine` loop the port mirrors
        (`C4Game.cpp:3239-3266`), so the two engines swap freely on the
        joining side once the config reaches the oracle the way it reads it.
        """

        ports = MODULE.RoundPorts(
            host_tcp=21500,
            host_udp=21501,
            reference=21502,
            client_tcp=21510,
            client_udp=21511,
        )
        command = MODULE.build_client_command(
            binary=Path("/oracle/clonk"),
            config=Path("/run/client/config.ini"),
            player_name="ClientPlayer",
            profile=Path("/run/profiles/ClientPlayer.c4p"),
            ports=ports,
            engine="cpp",
        )

        self.assertIn("/config:/run/client/config.ini", command)
        self.assertNotIn("--config", command)
        self.assertIn("/join:127.0.0.1:21502", command)
        self.assertIn("/tcpport:21510", command)
        self.assertIn("/udpport:21511", command)

    def test_the_oracle_host_takes_its_config_and_player_as_engine_parameters(self):
        """The oracle ignores the port's long options rather than refusing them.

        `C4Application::DoInit` reads the config path only from `/config:`
        (`C4Application.cpp:86`), and `C4Game::ParseCommandLine` skips every
        parameter it does not recognise (`C4Game.cpp:3141-3292`). Handing the
        oracle `--config <path>` therefore leaves it on its compiled-in
        defaults — the standard reference port rather than the round's — and
        the two peers never meet, with nothing in either log to say why.
        """

        ports = MODULE.RoundPorts(
            host_tcp=21500,
            host_udp=21501,
            reference=21502,
            client_tcp=21510,
            client_udp=21511,
        )
        command = MODULE.build_host_command(
            binary=Path("/oracle/clonk"),
            config=Path("/run/host/config.ini"),
            player_name="HostPlayer",
            profile=Path("/run/profiles/HostPlayer.c4p"),
            scenario=Path("/oracle/Melees.c4f/Massif.c4s"),
            ports=ports,
            engine="cpp",
        )

        self.assertIn("/config:/run/host/config.ini", command)
        self.assertIn("/run/profiles/HostPlayer.c4p", command)
        self.assertNotIn("--config", command)
        self.assertNotIn("--headless", command)
        self.assertNotIn("--player-name", command)
        # The rest of the line is the shared engine vocabulary.
        for token in ("/network", "/lobby", "/console", "/nosignup"):
            self.assertIn(token, command)
        self.assertIn("/tcpport:21500", command)

    def test_every_peer_keeps_its_stdin_open(self):
        """A console engine quits the moment stdin reports end of file.

        `CStdApp::ReadStdInCommand` returns false when `read` does not deliver
        a byte, and the caller turns that into `HR_Failure`
        (`StdAppUnix.cpp:414-455,581-596`), so the process exits successfully
        having played nothing. Redirecting a peer from `/dev/null` therefore
        produces a one-second run that looks like a clean startup.
        """

        with mock.patch.object(MODULE.subprocess, "Popen") as popen:
            MODULE.launch_peer(
                command=["/engine/clonk-app"],
                log=Path("/run/host.log"),
                environment={},
                working_directory=Path("/install"),
                opener=lambda _path: mock.sentinel.log_handle,
            )

        self.assertEqual(popen.call_args.kwargs["stdin"], subprocess.PIPE)
        self.assertTrue(popen.call_args.kwargs["text"])


class ConfigTests(unittest.TestCase):
    def test_each_peer_is_named_and_kept_off_discovery_and_the_masterserver(self):
        """Two unnamed peers on one machine collide on the machine name.

        Discovery is off for the same reason the ports are allocated per round:
        a run must reach the host it was pointed at and nothing else that
        happens to be listening on the box.
        """

        host = MODULE.render_process_config(
            name="RustHost", tcp=21500, udp=21501, reference=21502
        )
        client = MODULE.render_process_config(
            name="RustClient", tcp=21510, udp=21511, reference=21512
        )

        self.assertIn("LocalName=RustHost", host)
        self.assertIn("Nick=RustHost", host)
        self.assertIn("LocalName=RustClient", client)
        self.assertIn("PortDiscovery=0", host)
        self.assertIn("MasterServerSignUp=false", host)
        self.assertIn("LeagueServerSignUp=false", host)
        self.assertIn("PortRefServer=21502", host)


class ScenarioGuardTests(unittest.TestCase):
    def test_a_scenario_outside_the_install_root_is_rejected(self):
        """Out-of-root hosting silently loads a different world.

        The scenario's own folder becomes an install root, so the "installed"
        `Material.c4g` resolves back to the overlay the folder chain already
        contributed and the global group is never opened. Textures go missing
        from the host's map only, which reads as a desync between peers rather
        than as the loading fault it is.
        """

        install_root = Path("/install")

        MODULE.validate_scenario_inside_install_root(
            scenario=install_root / "content" / "Melees.c4f" / "Massif.c4s",
            install_root=install_root,
        )

        with self.assertRaisesRegex(MODULE.HarnessError, "install root"):
            MODULE.validate_scenario_inside_install_root(
                scenario=Path("/elsewhere/Massif.c4s"),
                install_root=install_root,
            )

    def test_a_scenario_reached_through_a_link_stays_inside_the_root(self):
        """The engine walks the path it was handed, links and all.

        Both engines derive the install root from the scenario path's own
        ancestors, and the oracle reaches every group through a symlink into a
        working checkout. Resolving the path first would move the scenario out
        of the root the engine will actually use and reject a correct setup —
        while the engine, given the resolved path, really would pick the other
        root.
        """

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            checkout = root / "checkout" / "content" / "Melees.c4f"
            checkout.mkdir(parents=True)
            (checkout / "Massif.c4s").mkdir()

            install_root = root / "oracle"
            install_root.mkdir()
            (install_root / "Melees.c4f").symlink_to(checkout)

            MODULE.validate_scenario_inside_install_root(
                scenario=MODULE.engine_path(
                    install_root / "Melees.c4f" / "Massif.c4s"
                ),
                install_root=MODULE.engine_path(install_root),
            )

    def test_a_single_player_scenario_is_rejected_before_anything_starts(self):
        """`MaxPlayer=1` produces a round that looks like a client defect.

        The host takes the only slot, the joining peer holds no player, and
        the engine deactivates it at `C4NetDeactivationDelay`
        (`C4Network2Client.cpp:648-654`). Reading the declared limit first is
        cheaper than reading that outcome wrong.
        """

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            solo = root / "Solo.c4s"
            solo.mkdir()
            (solo / "Scenario.txt").write_text(
                "[Head]\nTitle=Solo\nMaxPlayer=1\n", encoding="ascii"
            )
            melee = root / "Melee.c4s"
            melee.mkdir()
            (melee / "Scenario.txt").write_text(
                "[Head]\nTitle=Melee\nMaxPlayer=12\n", encoding="ascii"
            )

            self.assertEqual(MODULE.scenario_max_player(melee), 12)
            MODULE.require_multiplayer_scenario(melee)
            with self.assertRaisesRegex(MODULE.HarnessError, "MaxPlayer"):
                MODULE.require_multiplayer_scenario(solo)


class DirectionTests(unittest.TestCase):
    def test_a_mixed_direction_without_an_oracle_is_refused(self):
        """Falling back to the port would relabel the control run as mixed.

        The control and the two mixed directions differ only in which binary
        each peer runs, and every round prints the same summary either way. A
        direction that quietly ran port-against-port would answer this issue's
        question with the answer it already has.
        """

        with self.assertRaisesRegex(MODULE.HarnessError, "oracle"):
            MODULE.resolve_peers(
                direction="cpp-host",
                port_binary=Path("/engine/clonk-app"),
                port_root=Path("/repository"),
                oracle_binary=None,
            )

    def test_the_oracle_is_rooted_where_its_groups_are(self):
        """A `USE_CONSOLE` build reads its groups from beside the executable.

        That is a property of the build rather than a convention, so the root
        is derived from the binary instead of asked for — one fewer argument
        to get wrong, and the wrong value here is a silent content difference
        rather than a startup failure.
        """

        host, client = MODULE.resolve_peers(
            direction="cpp-client",
            port_binary=Path("/engine/clonk-app"),
            port_root=Path("/repository"),
            oracle_binary=Path("/oracle/build-console/clonk"),
        )

        self.assertEqual(host.engine, MODULE.RUST)
        self.assertEqual(host.install_root, Path("/repository"))
        self.assertEqual(client.engine, MODULE.CPP)
        self.assertEqual(client.binary, Path("/oracle/build-console/clonk"))
        self.assertEqual(client.install_root, Path("/oracle/build-console"))

    def test_the_oracle_is_named_relative_to_the_directory_it_runs_in(self):
        """The engine throws away the working directory it was given.

        `main` opens on macOS with
        `chdir(dirname(dirname(dirname(dirname(argv[0])))))`
        (`C4WinMain.cpp:231-239`) to climb out of `Clonk.app/Contents/MacOS/`.
        A console build is not in a bundle, so an absolute name walks four
        real directories out of the install root: measured on the pinned
        build, `/…/legacyclonk-oracle-pin/build-console/clonk` lands the
        process in `/Users/…/Documents/code`.

        The engine then says only `Error opening system group file
        (System.c4g)!` and carries on into a run that never opens a lobby —
        from outside, a host that failed to bring networking up.
        """

        _, client = MODULE.resolve_peers(
            direction="cpp-client",
            port_binary=Path("/repository/target/play/clonk-app"),
            port_root=Path("/repository"),
            oracle_binary=Path("/oracle/build-console/clonk"),
        )

        self.assertEqual(client.argv0, "./clonk")

    def test_the_port_keeps_the_binary_path_it_was_given(self):
        """The port's binary sits under its root rather than in it.

        Only a peer whose executable lives *in* its install root can name
        itself relative to it, so the control run is untouched by this.
        """

        host, _ = MODULE.resolve_peers(
            direction="control",
            port_binary=Path("/repository/target/play/clonk-app"),
            port_root=Path("/repository"),
            oracle_binary=None,
        )

        self.assertEqual(host.argv0, "/repository/target/play/clonk-app")

    def test_the_control_direction_runs_the_port_on_both_sides(self):
        """The control is still one binary and one root, as it was."""

        host, client = MODULE.resolve_peers(
            direction="control",
            port_binary=Path("/engine/clonk-app"),
            port_root=Path("/repository"),
            oracle_binary=None,
        )

        self.assertEqual((host.engine, client.engine), (MODULE.RUST, MODULE.RUST))
        self.assertEqual(host.install_root, client.install_root)


class SharedContentTests(unittest.TestCase):
    def test_two_roots_serving_different_shared_groups_are_rejected(self):
        """A mixed round needs two install roots, and they must agree.

        Each engine resolves its groups from its own root — the port from the
        repository, the oracle from the directory its executable sits in — so
        a mixed round is the first configuration where the two can disagree.
        When they do, the peers build different worlds from the same scenario
        and the round ends in what looks like a simulation divergence.
        """

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            shared = root / "shared"
            (shared / "planet").mkdir(parents=True)
            (shared / "content").mkdir(parents=True)
            (shared / "planet" / "System.c4g").write_text("system", encoding="ascii")
            (shared / "content" / "Material.c4g").write_text("material", encoding="ascii")

            oracle = root / "oracle"
            oracle.mkdir()
            (oracle / "System.c4g").symlink_to(shared / "planet" / "System.c4g")
            (oracle / "Material.c4g").symlink_to(shared / "content" / "Material.c4g")

            MODULE.validate_shared_content_roots(
                host_root=shared, client_root=oracle
            )

            # The same layout with one group of its own is not the same content.
            divergent = root / "divergent"
            divergent.mkdir()
            (divergent / "System.c4g").write_text("other system", encoding="ascii")
            (divergent / "Material.c4g").symlink_to(shared / "content" / "Material.c4g")

            with self.assertRaisesRegex(MODULE.HarnessError, "System.c4g"):
                MODULE.validate_shared_content_roots(
                    host_root=shared, client_root=divergent
                )

    def test_two_separate_copies_of_the_same_content_are_accepted(self):
        """What has to match is the bytes, not the path they arrive by.

        A worktree carries its own `content/` while the oracle's links point
        at the main checkout, so the two roots reach the same submodule commit
        through different files. Comparing paths would reject every mixed run
        outside the main checkout — a guard that fires on the normal case
        teaches people to remove it.
        """

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            first, second = root / "worktree", root / "checkout"
            for copy in (first, second):
                (copy / "planet" / "System.c4g").mkdir(parents=True)
                (copy / "content" / "Material.c4g").mkdir(parents=True)
                (copy / "planet" / "System.c4g" / "Rule.c4d").write_text(
                    "rule", encoding="ascii"
                )
                (copy / "content" / "Material.c4g" / "Earth.c4m").write_text(
                    "earth", encoding="ascii"
                )

            MODULE.validate_shared_content_roots(
                host_root=first, client_root=second
            )

            (second / "content" / "Material.c4g" / "Earth.c4m").write_text(
                "sand", encoding="ascii"
            )
            with self.assertRaisesRegex(MODULE.HarnessError, "Material.c4g"):
                MODULE.validate_shared_content_roots(
                    host_root=first, client_root=second
                )

    def test_a_dangling_group_link_is_named_rather_than_played_through(self):
        """The oracle's group links point outside its own tree.

        They have pointed into a deleted worktree before, which leaves the
        peer with no group where it expects one. The engine does not stop for
        that, so the round runs and only the comparison at the end is wrong —
        or, worse, is not.
        """

        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            shared = root / "shared"
            (shared / "planet").mkdir(parents=True)
            (shared / "content").mkdir(parents=True)
            (shared / "planet" / "System.c4g").write_text("system", encoding="ascii")
            (shared / "content" / "Material.c4g").write_text("material", encoding="ascii")

            oracle = root / "oracle"
            oracle.mkdir()
            (oracle / "System.c4g").symlink_to(root / "deleted-worktree" / "System.c4g")
            (oracle / "Material.c4g").symlink_to(shared / "content" / "Material.c4g")

            with self.assertRaisesRegex(MODULE.HarnessError, "System.c4g"):
                MODULE.validate_shared_content_roots(
                    host_root=shared, client_root=oracle
                )


if __name__ == "__main__":
    unittest.main()
