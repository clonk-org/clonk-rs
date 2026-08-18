# Clonk Rust

Clonk Rust is a 2D side-view action and strategy game. You command a small crew
of Clonks in a fully destructible landscape: dig for ore and gold, build
settlements and production chains, sail, fly, fight, and blow large holes in the
scenery. Landscapes deform under explosions, water flows, and material falls —
so terrain is a tool as much as a backdrop.

![A mountain scenario: peaks behind a landscape veined with gold and ore](docs/images/gameplay.png)

Up to four players can share one keyboard, or play together over LAN and the
internet in lockstep multiplayer.

## Installing

Download the release for your platform from the
[Releases](https://github.com/clonk-org/clonk-rs/releases) page:

| Platform | Asset |
| --- | --- |
| macOS (Apple silicon / Intel) | `.dmg` containing `Clonk Rust.app` |
| Windows | `-setup.exe` installer |
| Linux (x86_64) | `.zip` |

Each download already contains the game data; no separate content install is
needed.

## Playing

![The Clonk Rust startup menu](docs/images/startup-menu.png)

Start the game and pick a scenario from the startup menu. What ships with it:

- **Tutorial** — ten scenarios covering movement, digging, construction,
  and combat.
- **Worlds** — settlement scenarios on randomly generated maps, rising in
  difficulty. Mine resources, grow a village, keep the supply chain running.
- **Missions** and **Races** — objective-driven single- and multiplayer maps.
- **Melees** — competitive arenas for local or network play.
- **Themed packs** — Knights, Western, Fantasy, Far Worlds, Hazard,
  Clonk Mars, and Eke Reloaded.

![The scenario book listing the installed scenario folders](docs/images/scenario-selection.png)

Options cover controls (four keyboard sets and four gamepad sets), graphics
(resolution and scaling), and network settings. The in-game menu holds ten
save-game slots.

### Network voice chat

Network games between Clonk Rust clients support voice in the network lobby and
proximity voice during play. It is off by default: open **Options → Audio** and
tick **Enable voice chat**. Nothing opens the microphone until you do, whatever
the rest of this section says. In a running game, a speaker icon appears above
the Clonk each participant currently has selected. Enable voice before hosting
or joining; transport negotiation is fixed for that network connection.

The Audio sheet carries the settings you are likely to change — the opt-in,
the playback volume, the push-to-talk key, and **Open mic (voice activated)**.
The full set, including the two tuning values below, lives in **Options →
Advanced** under **Voice**, which is also where to configure voice at
resolutions too small for the sheet's Voice chat group (640x480).

Once enabled, `ActivationMode` chooses how the microphone opens:

- `PushToTalk` (the default, and **Open mic** left unticked) opens it only
  while a key is held. That key is the backquote (`` ` ``) unless you change
  `PushToTalkKey`, which the Audio sheet's **Push to talk** button rebinds.
- `VoiceActivated` (**Open mic** ticked) opens it whenever you could speak —
  with the window focused, either as a joined lobby client or in a running game
  with a Clonk selected — and transmits only while it hears you.
  `ActivationThreshold` (0–100) sets how loud that has to be, spread
  evenly over −60…0 dBFS rather than over raw amplitude, so the useful
  settings sit in the middle of the range rather than bunched at the bottom;
  `0` transmits continuously and `100` never opens. `ActivationHangover` is
  how many milliseconds it keeps transmitting after you stop, so word endings
  are not clipped.

`Volume` scales incoming speech in either mode.

**Options → Advanced → Voice → InputDevice** chooses the microphone.
**System default** asks CPAL to use the operating system's default input and
follows route changes the host reports. Choosing a named entry persists its
host-provided endpoint ID rather than its display name, so identically named
entries remain distinct. Clonk never substitutes a different endpoint ID: if
the selected ID disappears, capture waits, and resumes when that ID returns.
This is an endpoint guarantee, not a universal physical-device serial number.
CoreAudio and WASAPI normally provide stable device or endpoint IDs; Linux ALSA
may expose virtual routing aliases and card-index IDs, which can follow another
physical microphone or be reassigned when hardware order changes. Device or
route changes affect only the local capture source: remote voice playback and
deterministic game state are left untouched.

What the microphone picks up is cleaned up before it is sent, and each stage
can be switched off on its own under **Options → Advanced → Voice**:

- `EchoCancellation` subtracts what this machine is playing from what its
  microphone hears, so speakers do not send the game — or the other players —
  back out. Headphones make it unnecessary; on speakers it is what stops
  everyone else hearing themselves.
- `NoiseSuppression` holds down the part of the capture that does not change
  while you talk: fans, hum, traffic outside.
- `AutomaticGainControl` brings every talker to the same loudness, so a distant
  microphone is not drowned out by the person sitting on top of theirs.

All three start on. They follow the microphone opt-in rather than adding
another one, and none of them changes the fixed 20 ms frames voice travels in.

Lobby speech is centered and plays at the configured volume, independent of
players or crew. During play, speech fades linearly over 700 landscape pixels.
Terrain does not block it, so a wall or cave between two Clonks has no effect
beyond their distance. Voice uses best-effort UDP between peers that positively
negotiate support; older LegacyClonk clients remain compatible but silent.

Voice packets are encrypted. Each connection agrees its own key over the game's
reliable control stream, so somebody watching the network cannot listen in, and
a peer that cannot agree a key gets no voice at all rather than an unencrypted
fallback. That key exchange is not tied to any identity, so it does not defend
against somebody who can actively rewrite your game traffic — the same
limitation the rest of the network protocol has. Treat voice as private from
onlookers, not as a secure channel.

Tracker music needs the optional libxmp 4 runtime; install it through your
platform package manager, or point `LC_LIBXMP_LIBRARY` at a compatible library.
Everything else works without it.

MIDI music additionally needs FluidSynth 2 and a General MIDI SoundFont, which
no platform ships by default. Install the runtime through your package manager
(`fluidsynth` plus `soundfont-fluid` on Arch, `libfluidsynth3` plus
`fluid-soundfont-gm` on Debian/Ubuntu, `fluid-synth` in Homebrew) or point
`LC_FLUIDSYNTH_LIBRARY` at a compatible library. Put an `.sf2` or `.sf3` bank
in `~/Library/Audio/Sounds/Banks` on macOS, `/usr/share/soundfonts` on other
Unix systems, or a `soundfonts` folder beside the executable on Windows — or
name one explicitly through `SDL_SOUNDFONTS`. The synthesizer is also loaded
from beside the executable, matching tracker-music discovery.

## Building from source

The game data lives in a submodule, so clone recursively:

```sh
git clone --recurse-submodules https://github.com/clonk-org/clonk-rs.git
cd clonk-rs
cargo run --release -p clonk-app --bin clonk-app
```

For an existing clone, run `git submodule update --init --recursive` first.
GitHub's generated "Source code" archives do not include submodules and have no
runnable content tree — clone with Git, or use a release download.

To build a distributable package for the current platform:

```sh
cargo xtask package
```

On macOS a release ships one universal `.app`. That needs both architectures
installed (`rustup target add aarch64-apple-darwin x86_64-apple-darwin`); with
only the host one, the command logs a warning and packages a
single-architecture build named after the host triple.

Contributor documentation lives in [`AGENTS.md`](AGENTS.md). Remaining port
work is tracked in
[the issue tracker](https://github.com/clonk-org/clonk-rs/issues).

## Licensing

The Rust source in this workspace is available under the MIT license in
[`COPYING`](COPYING). That does **not** relicense the bundled game data:
graphics, audio, scripts, text, and other assets under [`planet/`](planet/) and
the [`content/`](content/) submodule remain under their own `COPYING` files,
including the Clonk content license carried by the content submodule.

The Eke Reloaded and Clonk Mars packs are redistributed under separate,
specifically granted permission, and the Metal & Magic and Queron 3 packs come
from CCAN — none of them under the source or general content licenses. Establish
the position for yourself before redistributing any of them. Third-party Rust
dependencies retain their own licenses.
