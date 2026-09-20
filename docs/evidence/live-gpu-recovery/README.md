# Live GPU recovery qualification

Captured by [Main validation run 35503634653](https://github.com/clonk-org/clonk-rs/actions/runs/35503634653)
from source `496ae5bd269dd3622d529f73f1266023e15d9eb9` on 2026-09-20.
The reports, images, logs, and player fixtures retained here are byte-for-byte
copies of the workflow artifacts. Every row used a release build and pinned
content `9a01c8f55f0fbdccfa2dcf3a67e3cfcfcac7c009`.

Original artifacts: [Linux](https://github.com/clonk-org/clonk-rs/actions/runs/35503634653/artifacts/10603167305),
[macOS](https://github.com/clonk-org/clonk-rs/actions/runs/35503634653/artifacts/10603381988),
and [Windows](https://github.com/clonk-org/clonk-rs/actions/runs/35503634653/artifacts/10603137677).

| Backend | OS | Adapter | Driver | Recovery |
| --- | --- | --- | --- | --- |
| [vulkan](vulkan/report.json) | Linux-6.17.0-1022-azure-x86_64-with-glibc2.39 | llvmpipe (LLVM 20.1.2, 256 bits) | llvmpipe / Mesa 25.2.8-0ubuntu0.24.04.2 (LLVM 20.1.2) | 188 ms |
| [gl](gl/report.json) | Linux-6.17.0-1022-azure-x86_64-with-glibc2.39 | llvmpipe (LLVM 20.1.2, 256 bits) | 4.5 (Core Profile) Mesa 25.2.8-0ubuntu0.24.04.2 | 335 ms |
| [metal](metal/report.json) | macOS-26.6.2-arm64-arm-64bit-Mach-O | Apple Paravirtual device | Not reported by wgpu | 98 ms |
| [dx12](dx12/report.json) | Windows-2025Server-10.0.26100-SP0 | Microsoft Basic Render Driver | 10.0.26100.33296 | 930 ms |

Linux uses Mesa software GPU adapters through Xvfb; Windows uses the DX12
WARP software adapter; macOS uses Apple's paravirtual Metal device. These
exercise the named retained-wgpu backends, not the separate wgpu-free
software presenter. They do not qualify physical GPU resets, hardware vendor
drivers, or every gameplay shader.

Every row presented 30 retained frames, destroyed the live device, observed
the `Destroyed` callback, released the old surface before replacement
(`surface_counts_at_drop: [1, 0]`), advanced renderer generation 1 to 2, and
presented three frames after recovery. The first recovered presentation
recreated and fully uploaded every source texture used by the reference scene
and matched its RGBA pixels exactly. Cached art from earlier screens is
recorded separately and restored only when it is used again. The fixture is
the static startup menu with a real player profile, so intentional animation
cannot mask a recovery failure. These timings describe this qualification run;
they are not a gameplay performance benchmark.

| Backend | Textures restored | Full upload bytes | Executable SHA-256 |
| --- | ---: | ---: | --- |
| vulkan | 66 | 3751840 | `191ed5f22defb0af8417d6342529d359f1dc300fbf312817ae57ea3f6019d102` |
| gl | 66 | 3751840 | `191ed5f22defb0af8417d6342529d359f1dc300fbf312817ae57ea3f6019d102` |
| metal | 66 | 3751840 | `8361c612e35e6c028c66642473d81814980688901dfd5c86d783075e87f73114` |
| dx12 | 67 | 4001440 | `62049f7502b93da66cbede56e72999bacf978bd97cdd34918964db7b77d54d70` |

Each `qualification.json` records the source and content commits, source
hashes, OS, compiler, build flags, executable checksum, and checksums of the
retained artifacts. Windows uses the shipped MSVC build configuration and
passes `validate-msvc-runtime.sh`.

Recheck the reports and artifact hashes from the repository root using
Python 3.11+ with Pillow:

```python
import hashlib
import json
from pathlib import Path
import sys
sys.path.insert(0, "scripts")
import run_device_loss_probe as probe

root = Path("docs/evidence/live-gpu-recovery")
for backend in ("vulkan", "gl", "dx12", "metal"):
    directory = root / backend
    probe.check_report(directory / "report.json", backend)
    qualification = json.loads((directory / "qualification.json").read_text())
    for name, expected in qualification["artifacts"].items():
        assert hashlib.sha256((directory / name).read_bytes()).hexdigest() == expected
```

See [the reproduction procedure](../../DEVICE_LOSS_QUALIFICATION.md).
