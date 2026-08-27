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


if __name__ == "__main__":
    unittest.main()
