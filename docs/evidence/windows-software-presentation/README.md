# Windows software presentation qualification

Captured by [Main validation run 35496930705](https://github.com/clonk-org/clonk-rs/actions/runs/35496930705)
on 2026-09-20. The [original artifact](https://github.com/clonk-org/clonk-rs/actions/runs/35496930705/artifacts/10601027635)
also includes the isolated configuration and native screenshot directories.
These retained reports, images and logs are byte-for-byte copies of that artifact.

| Property | Value |
| --- | --- |
| Source commit | `9c774f82275f4595938ad950e7f9b71fc159280d` |
| Content commit | `9a01c8f55f0fbdccfa2dcf3a67e3cfcfcac7c009` |
| OS | Windows Server 2025, `10.0.26100` |
| Architecture / target | AMD64 / `x86_64-pc-windows-msvc` |
| Window backend | Win32 (`display_backend: windows`) |
| Build | Release, Rust 1.98.1 / LLVM 22.1.8, static CRT, LLD ThinLTO |
| Executable SHA-256 | `477c8c02105e9ebe7d694537b1363f47f9c2516bd615a55ec06f7638c7679f7e` |

The workflow's `validate-msvc-runtime.sh` step passed. Its executable checksum
matches both qualification records, binding the probe to the validated shipped
build. Source inputs were clean and unchanged throughout both runs.
The hosted runner had no audio output device, so the application's existing
inert audio fallback was used.

Both [forced](forced/report.json) and [automatic](automatic/report.json) modes
passed startup, presentation, an OS resize from 800×600 to 760×560, native pointer
mapping from physical `(128, 96)` to GUI `(64, 48)` at scale 2, F9 screenshot
saving, 200×150 save-thumbnail encoding, and empty-window-registry shutdown.
The forced mode made zero GPU attempts. Automatic fallback made two attempts
with empty backend sets, then selected software with reason `no-adapter`.

`qualification.json` in each directory records the platform, build flags,
compiler, source/binary hashes, and SHA-256 of every retained report, image and
log. Recheck the reports and hashes from the repository root with Python 3.11+
and Pillow:

```python
import importlib.util
import json
from pathlib import Path

spec = importlib.util.spec_from_file_location("smoke", "scripts/run_software_presentation_smoke.py")
smoke = importlib.util.module_from_spec(spec)
spec.loader.exec_module(smoke)
root = Path("docs/evidence/windows-software-presentation")
for mode in ("forced", "automatic"):
    directory = root / mode
    smoke.check_report(directory / "report.json", check_input=True,
                       automatic_fallback=mode == "automatic", expected_backend="windows")
    qualification = json.loads((directory / "qualification.json").read_text())
    for name, expected in qualification["artifacts"].items():
        assert smoke.file_digest(directory / name) == expected, (mode, name)
```

The `fullscreen` report phase exercises drawable scaling and clipping; it is
not an OS fullscreen transition. The thumbnail check covers the production
encoder, not full savegame parity. This reference run does not measure gameplay
cadence, qualify other Windows versions, or exercise recovery from runtime GPU
device loss. See [the reproduction procedure](../../../scripts/SOFTWARE_PRESENTATION_SMOKE.md).
