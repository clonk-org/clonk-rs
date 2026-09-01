import copy
import functools
import hashlib
import importlib.util
import json
import shutil
import stat
import struct
import subprocess
import tempfile
import unittest
import zlib
from pathlib import Path
from unittest import mock


REPOSITORY = Path(__file__).resolve().parents[2]
SCRIPT = REPOSITORY / "scripts/acquire_presentation_oracle.py"
SPEC = importlib.util.spec_from_file_location("acquire_presentation_oracle", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


EXPECTED_CASE_IDS = (
    "startup-main",
    "startup-scenario-selection",
    "startup-network-browser",
    "startup-player-selection",
    "startup-options",
    "startup-about",
    "network-lobby",
    "loader",
    "hud",
    "ingame-menu",
    "object-menu",
    "gameplay",
    "evaluation",
)
EXPECTED_LAYOUT_IDS = frozenset(
    (*EXPECTED_CASE_IDS[:6], "hud", "ingame-menu", "object-menu", "gameplay", "evaluation")
)
EXPECTED_PORT_ASSET_EXEMPTIONS = {
    "startup-main": {
        "startup/main/branding/logo": "branding",
        "startup/main/branding/version": "branding",
        "startup/main/branding/fan-project": "branding",
    },
    "startup-scenario-selection": {
        "startup/scenario-selection/background": "super-resolved-startup-art",
    },
    "startup-network-browser": {
        "startup/network-browser/background": "super-resolved-startup-art",
    },
    "startup-player-selection": {
        "startup/player-selection/background": "super-resolved-startup-art",
    },
    "startup-options": {
        "startup/options/tabs/paper": "super-resolved-startup-art",
    },
    "startup-about": {
        "startup/about/branding/fan-project": "branding",
    },
    **{
        case_id: {"game/upper-board/branding/logo": "branding"}
        for case_id in ("hud", "ingame-menu", "object-menu", "gameplay", "evaluation")
    },
}


def run_git(repository, *arguments):
    return subprocess.run(
        ["git", "-C", str(repository), *arguments],
        check=True,
        capture_output=True,
        text=True,
    ).stdout.strip()


def initialise_repository(root):
    root.mkdir()
    run_git(root, "init", "--quiet")
    run_git(root, "config", "user.email", "presentation@example.invalid")
    run_git(root, "config", "user.name", "Presentation Test")


def commit_file(repository, relative, contents, message):
    path = repository / relative
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(contents, encoding="utf-8")
    run_git(repository, "add", relative)
    run_git(repository, "commit", "--quiet", "-m", message)
    return run_git(repository, "rev-parse", "HEAD")


def install_gitlink(repository, revision):
    run_git(
        repository,
        "update-index",
        "--add",
        "--cacheinfo",
        f"160000,{revision},content",
    )
    run_git(repository, "commit", "--quiet", "-m", "pin content")


def png_chunk(kind, payload):
    return (
        struct.pack(">I", len(payload))
        + kind
        + payload
        + struct.pack(">I", zlib.crc32(kind + payload) & 0xFFFFFFFF)
    )


@functools.lru_cache(maxsize=None)
def png_bytes(
    *,
    width=1280,
    height=720,
    bit_depth=8,
    color_type=2,
    interlace=0,
    filter_byte=0,
    illegal_filter_row=None,
    sample_byte=0,
    raw_adjustment=0,
    compressed_payload=None,
    idat_parts=1,
):
    payload = struct.pack(
        ">IIBBBBB", width, height, bit_depth, color_type, 0, 0, interlace
    )
    channels = 4 if color_type == 6 else 3
    scanline = bytes([filter_byte]) + bytes([sample_byte]) * (width * channels)
    raw = scanline * height
    if illegal_filter_row is not None:
        offset = illegal_filter_row * len(scanline)
        raw = raw[:offset] + b"\x05" + raw[offset + 1 :]
    if raw_adjustment < 0:
        raw = raw[:raw_adjustment]
    elif raw_adjustment > 0:
        raw += bytes(raw_adjustment)
    compressed = (
        compressed_payload
        if compressed_payload is not None
        else zlib.compress(raw, level=1)
    )
    split_at = max(1, len(compressed) // idat_parts)
    idats = []
    for offset in range(0, len(compressed), split_at):
        idats.append(png_chunk(b"IDAT", compressed[offset : offset + split_at]))
    return (
        MODULE.PNG_SIGNATURE
        + png_chunk(b"IHDR", payload)
        + b"".join(idats)
        + png_chunk(b"IEND", b"")
    )


def write_capture_set(root, *, suffix=b""):
    root.mkdir(parents=True)
    for case_id in EXPECTED_CASE_IDS:
        (root / f"{case_id}.png").write_bytes(png_bytes() + suffix)
    for case_id in EXPECTED_LAYOUT_IDS:
        (root / f"{case_id}.layout.json").write_text(
            json.dumps(
                {
                    "schema": MODULE.LAYOUT_SCHEMA,
                    "screen": case_id,
                    "resolution": "1280x720",
                    "scale": 100,
                    "elements": [
                        {
                            "path": f"startup/{case_id}/root",
                            "role": "dialog",
                            "rect": {"x": 0, "y": 0, "width": 1280, "height": 720},
                            "visible": True,
                            "caption": "LegacyClonk",
                            "lines": [
                                {
                                    "text": "LegacyClonk",
                                    "rect": {
                                        "x": 40,
                                        "y": 40,
                                        "width": 120,
                                        "height": 20,
                                    },
                                }
                            ],
                        },
                        *[
                            {
                                "path": element_path,
                                "role": "image",
                                "rect": {
                                    "x": 0,
                                    "y": 0,
                                    "width": 1280,
                                    "height": 720,
                                },
                                "visible": True,
                                "port_asset": asset,
                                "caption": "",
                                "lines": [],
                            }
                            for element_path, asset in EXPECTED_PORT_ASSET_EXEMPTIONS[
                                case_id
                            ].items()
                        ],
                    ],
                },
                sort_keys=True,
            ),
            encoding="utf-8",
        )


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def write_test_comparator(path):
    path.write_text(
        """#!/usr/bin/env python3
import json
import os
from pathlib import Path
import sys

case_id = os.environ["CLONK_PRESENTATION_CASE"]
reference = Path(os.environ["CLONK_PRESENTATION_REFERENCE"])
actual = Path(os.environ["CLONK_PRESENTATION_ACTUAL"])
layout_cases = {
    "startup-main",
    "startup-scenario-selection",
    "startup-network-browser",
    "startup-player-selection",
    "startup-options",
    "startup-about",
    "hud",
    "ingame-menu",
    "object-menu",
    "gameplay",
    "evaluation",
}
comparison = "layout" if case_id in layout_cases else "pixel"
if reference.read_bytes() != actual.read_bytes():
    print(f"{comparison} mismatch: synthetic comparator canary", file=sys.stderr)
    sys.exit(1)
print(json.dumps({"schema":"clonk-rs/presentation-comparison/v1","case_id":case_id,"comparison":comparison,"status":"match"}, separators=(",", ":")))
""",
        encoding="utf-8",
    )
    path.chmod(0o755)


def canonical_sha256(value):
    encoded = json.dumps(value, sort_keys=True, separators=(",", ":")).encode("utf-8")
    return hashlib.sha256(encoded).hexdigest()


def resource_manifest_sha256(entries):
    canonical = "".join(
        f"{hashlib.sha256(contents).hexdigest()}  {path}\n"
        for path, contents in sorted(entries.items())
    ).encode("utf-8")
    return hashlib.sha256(canonical).hexdigest()


def artifact_metadata(root, relative):
    path = root / relative
    return {
        "path": relative,
        "sha256": sha256(path),
        "size_bytes": path.stat().st_size,
    }


def expected_provenance_fields():
    return {
        "oracle_source_tree": "1" * 40,
        "fixture_content_tree": "2" * 40,
        "oracle_planet_tree": "3" * 40,
        "rust_planet_tree": "4" * 40,
        "oracle_system_tree": "5" * 40,
        "rust_system_tree": "6" * 40,
        "effective_system_scripts_sha256": "7" * 64,
        "runtime_resources": synthetic_runtime_resources(),
    }


def v2_expected_provenance_fields():
    return {
        **expected_provenance_fields(),
        "rust_source_commit": "8" * 40,
        "rust_source_tree": "9" * 40,
    }


def synthetic_runtime_resources():
    return {
        "cpp": copy.deepcopy(MODULE.PINNED_CPP_RUNTIME_RESOURCES),
        "rust": {
            "graphics": {"tree": "a" * 40, "manifest_sha256": "b" * 64},
            "system": {"tree": "c" * 40, "manifest_sha256": "d" * 64},
        },
    }


def synthetic_case_specs(configs, player_sha256, content_tree, runtime_resources):
    locale = {
        "language": "US",
        "charset": "Windows-1252",
        "lang": "C",
        "lc_all": "C",
        "tz": "UTC",
    }
    seeds = {
        "simulation": {"seed": 587, "calls": 0},
        "presentation": {
            "algorithm": MODULE.PRESENTATION_RNG_ALGORITHM,
            "seed": MODULE.PRESENTATION_RNG_SEED,
            "calls": 0,
            "trace_sha256": MODULE.PRESENTATION_RNG_EMPTY_TRACE_SHA256,
        },
    }
    return [
        {
            "id": case_id,
            "comparison": "layout" if case_id in EXPECTED_LAYOUT_IDS else "pixel",
            "port_asset_exemptions": copy.deepcopy(
                EXPECTED_PORT_ASSET_EXEMPTIONS.get(case_id, {})
            ),
            "config_sha256": {
                engine: configs[engine]["sha256"] for engine in ("cpp", "rust")
            },
            "player_sha256": player_sha256,
            "locale": locale,
            "seeds": seeds,
            "trigger": {"id": f"test/open/{case_id}"},
            "scenario": {"path": None, "content_tree": content_tree},
            "frame": {"checkpoint": f"test/ready/{case_id}", "number": 0},
            "runtime_resources": copy.deepcopy(runtime_resources),
        }
        for case_id in EXPECTED_CASE_IDS
    ]


def write_v2_candidate(root):
    root.mkdir()
    expected_fields = v2_expected_provenance_fields()
    trusted_patch = root.parent / "trusted-presentation-capture.patch"
    trusted_launcher = root.parent / "trusted-acquisition-launcher.py"
    trusted_patch.write_text("audited capture patch\n", encoding="utf-8")
    trusted_launcher.write_text("# audited launcher\n", encoding="utf-8")

    retained_patch = root / "inputs/cpp-capture.patch"
    retained_launcher = root / "launcher/acquire_presentation_oracle.py"
    retained_patch.parent.mkdir()
    retained_launcher.parent.mkdir()
    retained_patch.write_bytes(trusted_patch.read_bytes())
    retained_launcher.write_bytes(trusted_launcher.read_bytes())

    configs = {}
    for engine, profile in (("cpp", "oracle-native"), ("rust", "legacy-clonk")):
        relative = f"inputs/{engine}.config"
        path = root / relative
        path.write_bytes(
            (REPOSITORY / f"compat/presentation/{engine}.config").read_bytes()
        )
        configs[engine] = {
            **artifact_metadata(root, relative),
            "profile": profile,
    }
    player_path = root / MODULE.PLAYER_RETAINED_PATH
    player_path.write_bytes((REPOSITORY / MODULE.PLAYER_SOURCE_PATH).read_bytes())
    player = {
        **artifact_metadata(root, MODULE.PLAYER_RETAINED_PATH),
        "id": "presentation-player",
    }
    network_path = root / MODULE.NETWORK_REFERENCES_RETAINED_PATH
    network_path.write_bytes(
        (REPOSITORY / MODULE.NETWORK_REFERENCES_SOURCE_PATH).read_bytes()
    )
    network = artifact_metadata(root, MODULE.NETWORK_REFERENCES_RETAINED_PATH)
    source_entries = [
        {
            "path": "Cargo.toml",
            "mode": "100644",
            "git_oid": "c" * 40,
            "sha256": "d" * 64,
            "size_bytes": 1,
        }
    ]
    source_inventory = {
        "schema": MODULE.RUST_SOURCE_INVENTORY_SCHEMA,
        "algorithm": "sorted-git-blob-sha256-v1",
        "sha256": canonical_sha256(source_entries),
        "entries": source_entries,
    }
    source_inventory_path = root / MODULE.RUST_SOURCE_INVENTORY_RETAINED_PATH
    source_inventory_path.write_text(
        json.dumps(source_inventory, sort_keys=True),
        encoding="utf-8",
    )

    case_specs = synthetic_case_specs(
        configs,
        player["sha256"],
        expected_fields["fixture_content_tree"],
        expected_fields["runtime_resources"],
    )
    case_specs_sha256 = canonical_sha256(case_specs)
    builds = {}
    producers = {
        "cpp": "legacyclonk-capture-patch-v1",
        "rust": "clonk-rs-capture-driver-v1",
    }
    profiles = {"cpp": "oracle-native", "rust": "legacy-clonk"}
    source_trees = {
        "cpp": expected_fields["oracle_source_tree"],
        "rust": expected_fields["rust_source_tree"],
    }
    for engine in ("cpp", "rust"):
        binary_relative = f"builds/{engine}/binary"
        binary_path = root / binary_relative
        binary_path.parent.mkdir(parents=True)
        if engine == "rust":
            write_test_comparator(binary_path)
        else:
            binary_path.write_bytes(b"synthetic-cpp-binary\n")
        recipe = {
            "commands": [
                {
                    "argv": ["build", engine],
                    "cwd": f"work/{engine}-source",
                    "environment": {},
                }
            ]
        }
        builds[engine] = {
            "source_tree": source_trees[engine],
            "capture_patch_sha256": sha256(trusted_patch) if engine == "cpp" else None,
            "producer": producers[engine],
            "profile": profiles[engine],
            "recipe": {**recipe, "sha256": canonical_sha256(recipe)},
            "binary": artifact_metadata(root, binary_relative),
        }

    runs = []
    for run_index, nonce_character in enumerate(("a", "b"), start=1):
        run_id = f"run-{run_index}"
        nonce = nonce_character * 64
        engines = {}
        launcher_receipts = {}
        for engine in ("cpp", "rust"):
            cases = []
            launcher_receipts[engine] = []
            for spec in case_specs:
                case_id = spec["id"]
                artifact_directory = root / run_id / engine / "artifacts"
                artifact_directory.mkdir(parents=True, exist_ok=True)
                png_relative = f"{run_id}/{engine}/artifacts/{case_id}.png"
                (root / png_relative).write_bytes(
                    png_bytes(sample_byte=255 if case_id == "loader" else 0)
                )
                artifacts = {"png": artifact_metadata(root, png_relative)}
                if case_id in EXPECTED_LAYOUT_IDS:
                    layout_relative = (
                        f"{run_id}/{engine}/artifacts/{case_id}.layout.json"
                    )
                    (root / layout_relative).write_text(
                        json.dumps(
                            {
                                "schema": MODULE.LAYOUT_SCHEMA,
                                "screen": case_id,
                                "resolution": "1280x720",
                                "scale": 100,
                                "elements": [
                                    {
                                        "path": f"startup/{case_id}/root",
                                        "role": "dialog",
                                        "rect": {
                                            "x": 0,
                                            "y": 0,
                                            "width": 1280,
                                            "height": 720,
                                        },
                                        "visible": True,
                                        "caption": "LegacyClonk",
                                        "lines": [],
                                    },
                                    *[
                                        {
                                            "path": element_path,
                                            "role": "image",
                                            "rect": {
                                                "x": 0,
                                                "y": 0,
                                                "width": 1280,
                                                "height": 720,
                                            },
                                            "visible": True,
                                            "port_asset": asset,
                                            "caption": "",
                                            "lines": [],
                                        }
                                        for element_path, asset in (
                                            EXPECTED_PORT_ASSET_EXEMPTIONS[case_id].items()
                                        )
                                    ],
                                ],
                            },
                            sort_keys=True,
                        ),
                        encoding="utf-8",
                    )
                    artifacts["layout"] = artifact_metadata(root, layout_relative)
                receipt = {
                    "schema": MODULE.ENGINE_RECEIPT_SCHEMA,
                    "run_id": run_id,
                    "launcher_nonce": nonce,
                    "engine": engine,
                    "producer": producers[engine],
                    "case_id": case_id,
                    "binary_sha256": builds[engine]["binary"]["sha256"],
                    "source_tree": source_trees[engine],
                    "content_tree": spec["scenario"]["content_tree"],
                    "profile": profiles[engine],
                    "config_sha256": spec["config_sha256"][engine],
                    "player_sha256": spec["player_sha256"],
                    "network_references_sha256": network["sha256"],
                    "locale": spec["locale"],
                    "seeds": spec["seeds"],
                    "trigger": spec["trigger"],
                    "scenario": spec["scenario"],
                    "frame": spec["frame"],
                    "runtime_resources": copy.deepcopy(
                        spec["runtime_resources"][engine]
                    ),
                    "artifacts": artifacts,
                }
                receipt_relative = f"{run_id}/{engine}/receipts/{case_id}.json"
                receipt_path = root / receipt_relative
                receipt_path.parent.mkdir(parents=True, exist_ok=True)
                receipt_path.write_text(json.dumps(receipt, sort_keys=True), encoding="utf-8")
                receipt_metadata = artifact_metadata(root, receipt_relative)
                environment = MODULE._capture_environment(
                    root,
                    run_id=run_id,
                    nonce=nonce,
                    engine=engine,
                    case_id=case_id,
                )
                if engine == "cpp":
                    binary = str(
                        root
                        / "work/oracle-source/build/clonk.app/Contents/MacOS/clonk"
                    )
                    startup = MODULE.CPP_STARTUP_ARGUMENTS.get(case_id)
                    if startup is not None:
                        middle = [startup]
                    else:
                        scenario = (
                            root
                            / "work/fixture-content"
                            / MODULE.CPP_RUNTIME_SCENARIOS[case_id]
                        )
                        middle = [str(scenario)]
                        if case_id == "network-lobby":
                            middle.extend(["/network", "/lobby"])
                    argv = [
                        binary,
                        *middle,
                        f"/config:{root / 'inputs/cpp.config'}",
                    ]
                    cwd = root / "inputs"
                else:
                    argv = [
                        str(root / "work/rust-source/target/release/clonk-app"),
                        "--config",
                        str(root / "inputs/rust.config"),
                        str(root / MODULE.PLAYER_RETAINED_PATH),
                        "/compatprofile:legacy-clonk",
                    ]
                    cwd = root / "work/rust-source"
                cases.append(
                    {
                        "id": case_id,
                        "receipt": receipt_metadata,
                        "launch": {
                            "argv": argv,
                            "cwd": str(cwd),
                            "environment": environment,
                            "runtime_resources": copy.deepcopy(
                                spec["runtime_resources"][engine]
                            ),
                        },
                    }
                )
                launcher_receipts[engine].append(
                    {"case_id": case_id, "sha256": receipt_metadata["sha256"]}
                )
            engines[engine] = {"cases": cases}
        launcher_receipt = {
            "schema": MODULE.LAUNCHER_RECEIPT_SCHEMA,
            "run_id": run_id,
            "nonce": nonce,
            "launcher_sha256": sha256(trusted_launcher),
            "case_specs_sha256": case_specs_sha256,
            "engine_receipts": launcher_receipts,
            "launches_sha256": canonical_sha256(
                [
                    {
                        "engine": engine,
                        "case_id": case["id"],
                        **case["launch"],
                    }
                    for engine in ("cpp", "rust")
                    for case in engines[engine]["cases"]
                ]
            ),
        }
        launcher_relative = f"{run_id}/launcher-receipt.json"
        (root / launcher_relative).write_text(
            json.dumps(launcher_receipt, sort_keys=True), encoding="utf-8"
        )
        runs.append(
            {
                "id": run_id,
                "nonce": nonce,
                "launcher_receipt": artifact_metadata(root, launcher_relative),
                "engines": engines,
            }
        )

    comparisons = []
    for run in runs:
        run_id = run["id"]
        cases = []
        for case_id in EXPECTED_CASE_IDS:
            engine_artifacts = {}
            for engine in ("cpp", "rust"):
                case = next(
                    case
                    for case in run["engines"][engine]["cases"]
                    if case["id"] == case_id
                )
                receipt = json.loads(
                    (root / case["receipt"]["path"]).read_text(encoding="utf-8")
                )
                engine_artifacts[engine] = receipt["artifacts"]
            artifact_key = "layout" if case_id in EXPECTED_LAYOUT_IDS else "png"
            comparison_term = "layout" if case_id in EXPECTED_LAYOUT_IDS else "pixel"
            result = {
                "schema": MODULE.COMPARISON_RECEIPT_SCHEMA,
                "case_id": case_id,
                "comparison": comparison_term,
                "status": "match",
            }
            reference = copy.deepcopy(engine_artifacts["cpp"][artifact_key])
            actual = copy.deepcopy(engine_artifacts["rust"][artifact_key])
            receipt_relative = f"{run_id}/comparisons/{case_id}.json"
            receipt_path = root / receipt_relative
            receipt_path.parent.mkdir(parents=True, exist_ok=True)
            receipt_path.write_text(
                json.dumps(
                    {
                        "schema": MODULE.COMPARISON_ATTESTATION_SCHEMA,
                        "run_id": run_id,
                        "case_id": case_id,
                        "comparison": comparison_term,
                        "comparator": {
                            "schema": MODULE.COMPARISON_RECEIPT_SCHEMA,
                            "binary_sha256": builds["rust"]["binary"]["sha256"],
                        },
                        "reference": {
                            "path": reference["path"],
                            "sha256": reference["sha256"],
                        },
                        "actual": {
                            "path": actual["path"],
                            "sha256": actual["sha256"],
                        },
                        "stdout": json.dumps(result, separators=(",", ":")) + "\n",
                    },
                    sort_keys=True,
                ),
                encoding="utf-8",
            )
            cases.append(
                {
                    "id": case_id,
                    "comparison": comparison_term,
                    "reference": reference,
                    "actual": actual,
                    "receipt": artifact_metadata(root, receipt_relative),
                }
            )
        comparisons.append({"id": run_id, "cases": cases})

    capture_manifest = REPOSITORY / "compat/presentation_captures.json"
    index = {
        "schema": MODULE.PROVENANCE_SCHEMA,
        "contract": {
            "capture_manifest": {
                "path": "compat/presentation_captures.json",
                "contract_sha256": MODULE.capture_manifest_contract_sha256(
                    capture_manifest
                ),
            },
            "compat_profile": {
                "path": "compat/profile.json",
                "contract_sha256": MODULE.compat_profile_contract_sha256(
                    REPOSITORY / "compat/profile.json"
                ),
            },
            "case_specs": {
                "path": "compat/presentation/case_specs.json",
                "sha256": case_specs_sha256,
            },
            "geometry": {"width": 1280, "height": 720, "scale": 100},
            "normalization": dict(MODULE.EXPECTED_NORMALIZATION),
            "launcher": {
                "source_path": "scripts/acquire_presentation_oracle.py",
                "retained_path": "launcher/acquire_presentation_oracle.py",
                "sha256": sha256(trusted_launcher),
            },
        },
        "sources": {
            "oracle": {
                "commit": MODULE.ORACLE_SOURCE_COMMIT,
                "tree": expected_fields["oracle_source_tree"],
                "historical_content_gitlink": MODULE.ORACLE_SOURCE_CONTENT_GITLINK,
                "planet_tree": expected_fields["oracle_planet_tree"],
                "system_tree": expected_fields["oracle_system_tree"],
            },
            "rust": {
                "commit": expected_fields["rust_source_commit"],
                "tree": expected_fields["rust_source_tree"],
                "planet_tree": expected_fields["rust_planet_tree"],
                "system_tree": expected_fields["rust_system_tree"],
            },
            "fixture_content": {
                "commit": MODULE.FIXTURE_CONTENT_COMMIT,
                "tree": expected_fields["fixture_content_tree"],
            },
            "effective_system_scripts_sha256": expected_fields[
                "effective_system_scripts_sha256"
            ],
            "runtime_resources": copy.deepcopy(
                expected_fields["runtime_resources"]
            ),
        },
        "patch": {
            "source_path": "parity/oracle/presentation_capture.patch",
            "retained_path": "inputs/cpp-capture.patch",
            "base_commit": MODULE.ORACLE_SOURCE_COMMIT,
            "sha256": sha256(trusted_patch),
        },
        "inputs": {
            "configs": configs,
            "player": player,
            "network_references": network,
            "rust_source_inventory": artifact_metadata(
                root,
                MODULE.RUST_SOURCE_INVENTORY_RETAINED_PATH,
            ),
        },
        "builds": builds,
        "case_specs": case_specs,
        "runs": runs,
        "comparisons": comparisons,
    }
    (root / "index.json").write_text(json.dumps(index, sort_keys=True), encoding="utf-8")
    return {
        "index": index,
        "expected_fields": expected_fields,
        "case_specs": case_specs,
        "trusted_patch": trusted_patch,
        "trusted_launcher": trusted_launcher,
        "source_inventory": source_inventory,
        "trusted_comparator": root / builds["rust"]["binary"]["path"],
    }


def rewrite_engine_receipt(candidate, index, run_id, engine, case_id, mutate):
    run = next(entry for entry in index["runs"] if entry["id"] == run_id)
    case = next(
        entry for entry in run["engines"][engine]["cases"] if entry["id"] == case_id
    )
    receipt_path = candidate / case["receipt"]["path"]
    receipt = json.loads(receipt_path.read_text(encoding="utf-8"))
    mutate(receipt)
    receipt_path.write_text(json.dumps(receipt, sort_keys=True), encoding="utf-8")
    case["receipt"] = artifact_metadata(candidate, case["receipt"]["path"])

    launcher_path = candidate / run["launcher_receipt"]["path"]
    launcher = json.loads(launcher_path.read_text(encoding="utf-8"))
    launcher_entry = next(
        entry
        for entry in launcher["engine_receipts"][engine]
        if entry["case_id"] == case_id
    )
    launcher_entry["sha256"] = case["receipt"]["sha256"]
    launcher_path.write_text(json.dumps(launcher, sort_keys=True), encoding="utf-8")
    run["launcher_receipt"] = artifact_metadata(
        candidate, run["launcher_receipt"]["path"]
    )


def provenance_index(artifact_root):
    fields = expected_provenance_fields()
    captures = {}
    for case_id in EXPECTED_CASE_IDS:
        png_name = f"{case_id}.png"
        entry = {
            "png": {
                "path": png_name,
                "sha256": sha256(artifact_root / png_name),
            }
        }
        if case_id in EXPECTED_LAYOUT_IDS:
            layout_name = f"{case_id}.layout.json"
            entry["layout"] = {
                "path": layout_name,
                "sha256": sha256(artifact_root / layout_name),
            }
        captures[case_id] = entry
    return {
        "schema": MODULE.PROVENANCE_SCHEMA,
        "oracle_source_commit": MODULE.ORACLE_SOURCE_COMMIT,
        "oracle_source_content_gitlink": MODULE.ORACLE_SOURCE_CONTENT_GITLINK,
        "fixture_content_commit": MODULE.FIXTURE_CONTENT_COMMIT,
        **fields,
        "capture_patch_sha256": "8" * 64,
        "geometry": {"width": 1280, "height": 720, "scale": 100},
        "normalization": dict(MODULE.EXPECTED_NORMALIZATION),
        "captures": captures,
    }


class PinAndGitTests(unittest.TestCase):
    def test_run_bounds_a_command_and_reports_its_partial_output(self):
        timeout = subprocess.TimeoutExpired(
            ["headed-capture"],
            7,
            output="capture started\n",
            stderr="window never closed\n",
        )
        with mock.patch.object(MODULE.subprocess, "run", side_effect=timeout) as run:
            with self.assertRaisesRegex(
                MODULE.AcquisitionFailure,
                "timed out after 7 seconds.*window never closed",
            ):
                MODULE._run(
                    ["headed-capture"],
                    text=True,
                    timeout_seconds=7,
                )

        self.assertEqual(run.call_args.kwargs["timeout"], 7)

    def test_presentation_rng_contract_pins_darwin_vector_and_empty_trace(self):
        self.assertEqual(
            MODULE.darwin_park_miller_values(587, 5),
            (9_865_709, 456_730_344, 1_160_337_230, 488_826_203, 1_577_044_046),
        )
        valid = {
            "algorithm": "darwin-libc-rand-park-miller-v1",
            "seed": 587,
            "calls": 0,
            "trace_sha256": hashlib.sha256(b"").hexdigest(),
        }
        self.assertEqual(
            MODULE._validate_presentation_rng(valid, "presentation RNG"),
            valid,
        )
        mutations = {
            "algorithm": {**valid, "algorithm": "host-libc-rand"},
            "seed": {**valid, "seed": 588},
            "empty trace": {**valid, "trace_sha256": "0" * 64},
        }
        for label, value in mutations.items():
            with self.subTest(label=label):
                with self.assertRaises(MODULE.AcquisitionFailure):
                    MODULE._validate_presentation_rng(value, "presentation RNG")

    def test_clean_source_checkpoint_rejects_build_or_capture_mutation(self):
        with tempfile.TemporaryDirectory() as temporary:
            repository = Path(temporary) / "source"
            initialise_repository(repository)
            commit = commit_file(repository, "Cargo.toml", "[workspace]\n", "source")
            tree = run_git(repository, "rev-parse", "HEAD^{tree}")
            inventory = MODULE.rust_source_input_inventory(
                repository,
                commit,
                source_paths=("Cargo.toml",),
            )
            MODULE.verify_clean_source_checkpoint(
                repository,
                expected_commit=commit,
                expected_tree=tree,
                expected_inventory=inventory,
                label="after build",
                required_paths=("Cargo.toml",),
                source_paths=("Cargo.toml",),
            )

            (repository / "Cargo.toml").write_text(
                "[workspace]\nmembers=[]\n", encoding="utf-8"
            )
            with self.assertRaisesRegex(
                MODULE.AcquisitionFailure, "after capture.*source"
            ):
                MODULE.verify_clean_source_checkpoint(
                    repository,
                    expected_commit=commit,
                    expected_tree=tree,
                    expected_inventory=inventory,
                    label="after capture",
                    required_paths=("Cargo.toml",),
                    source_paths=("Cargo.toml",),
                )

    def test_runtime_resource_materialization_uses_raw_blobs_not_archive_eol_filters(self):
        with tempfile.TemporaryDirectory() as temporary:
            temporary = Path(temporary)
            repository = temporary / "source"
            initialise_repository(repository)
            (repository / ".gitattributes").write_text(
                "planet/System.c4g/*.txt text eol=crlf\n",
                encoding="utf-8",
            )
            resource = repository / "planet/System.c4g/LanguageUS.txt"
            resource.parent.mkdir(parents=True)
            resource.write_bytes(b"one\ntwo\n")
            run_git(repository, "add", ".gitattributes", "planet/System.c4g")
            run_git(repository, "commit", "--quiet", "-m", "resources")
            revision = run_git(repository, "rev-parse", "HEAD")
            expected = MODULE.runtime_resource_identity_at_revision(
                repository,
                revision,
                "planet/System.c4g",
            )

            archived = temporary / "archive"
            MODULE.materialize_git_archive(repository, revision, archived)
            self.assertNotEqual(
                MODULE.runtime_resource_identity(archived / "planet/System.c4g"),
                expected,
            )

            raw = temporary / "raw-system"
            MODULE.materialize_runtime_resource_tree(
                repository,
                revision,
                "planet/System.c4g",
                raw,
            )
            self.assertEqual(MODULE.runtime_resource_identity(raw), expected)
            self.assertEqual((raw / "LanguageUS.txt").read_bytes(), b"one\ntwo\n")

    def test_clean_source_revision_rejects_tracked_and_untracked_drift(self):
        with tempfile.TemporaryDirectory() as temporary:
            repository = Path(temporary) / "source"
            initialise_repository(repository)
            commit_file(repository, "required.txt", "committed\n", "source")

            commit, tree = MODULE.require_clean_source_revision(
                repository,
                required_paths=("required.txt",),
            )
            self.assertEqual(commit, run_git(repository, "rev-parse", "HEAD"))
            self.assertEqual(tree, run_git(repository, "rev-parse", "HEAD^{tree}"))

            (repository / "required.txt").write_text("dirty\n", encoding="utf-8")
            with self.assertRaisesRegex(MODULE.AcquisitionFailure, "tracked source drift"):
                MODULE.require_clean_source_revision(
                    repository,
                    required_paths=("required.txt",),
                )

    def test_clean_source_allows_only_the_established_top_level_update_lock(self):
        with tempfile.TemporaryDirectory() as temporary:
            repository = Path(temporary) / "source"
            initialise_repository(repository)
            commit_file(repository, "required.txt", "committed\n", "source")
            (repository / ".clonk-update.lock").write_text("lock\n", encoding="utf-8")

            MODULE.require_clean_source_revision(
                repository,
                required_paths=("required.txt",),
            )

            nested = repository / "nested/.clonk-update.lock"
            nested.parent.mkdir()
            nested.write_text("nested lock\n", encoding="utf-8")
            with self.assertRaisesRegex(MODULE.AcquisitionFailure, "untracked source"):
                MODULE.require_clean_source_revision(
                    repository,
                    required_paths=("required.txt",),
                )

    def test_rust_source_inventory_is_sorted_and_binds_only_selected_inputs(self):
        with tempfile.TemporaryDirectory() as temporary:
            repository = Path(temporary) / "source"
            initialise_repository(repository)
            commit_file(repository, "Cargo.toml", "[workspace]\n", "manifest")
            first = commit_file(repository, "crates/app/src/main.rs", "fn main() {}\n", "app")

            inventory = MODULE.rust_source_input_inventory(
                repository,
                first,
                source_paths=("Cargo.toml", "crates"),
            )
            MODULE.validate_rust_source_input_inventory(inventory, expected=inventory)
            self.assertEqual(
                [entry["path"] for entry in inventory["entries"]],
                ["Cargo.toml", "crates/app/src/main.rs"],
            )
            with self.assertRaisesRegex(MODULE.AcquisitionFailure, "not committed"):
                MODULE.rust_source_input_inventory(
                    repository,
                    first,
                    source_paths=("Cargo.toml", "missing-input.json"),
                )

            unrelated = commit_file(repository, "README.md", "docs\n", "docs")
            self.assertEqual(
                MODULE.rust_source_input_inventory(
                    repository,
                    unrelated,
                    source_paths=("Cargo.toml", "crates"),
                ),
                inventory,
            )
            changed = commit_file(
                repository,
                "crates/app/src/main.rs",
                "fn main() { println!(\"changed\"); }\n",
                "change app",
            )
            with self.assertRaisesRegex(MODULE.AcquisitionFailure, "source inputs differ"):
                MODULE.validate_rust_source_input_inventory(
                    inventory,
                    expected=MODULE.rust_source_input_inventory(
                        repository,
                        changed,
                        source_paths=("Cargo.toml", "crates"),
                    ),
                )
            (repository / "required.txt").write_text("committed\n", encoding="utf-8")
            (repository / "untracked.txt").write_text("untracked\n", encoding="utf-8")
            with self.assertRaisesRegex(MODULE.AcquisitionFailure, "untracked source files"):
                MODULE.require_clean_source_revision(
                    repository,
                    required_paths=("required.txt",),
                )

    def test_constants_pin_the_contract_source_content_and_case_inventory(self):
        self.assertEqual(
            MODULE.ORACLE_SOURCE_COMMIT,
            "7d43b47b7d789b533f32d005e64596e0a07019cd",
        )
        self.assertEqual(
            MODULE.ORACLE_SOURCE_CONTENT_GITLINK,
            "67a54d0e662bda3aa0202134efc065d7bc420872",
        )
        self.assertEqual(
            MODULE.FIXTURE_CONTENT_COMMIT,
            "ab9094f96838ae9c8cb77555560a8b887231640a",
        )
        self.assertEqual(MODULE.CASE_IDS, EXPECTED_CASE_IDS)
        self.assertEqual(MODULE.LAYOUT_CASE_IDS, EXPECTED_LAYOUT_IDS)

    def test_about_layout_uses_only_the_branding_port_asset_class(self):
        # Pinned C++ C4Startup.cpp:317 loads LoaderWatercave1 for About; that
        # byte-identical shared background is not the super-res startup-paper
        # replacement used by Options.
        self.assertEqual(
            MODULE.LAYOUT_PORT_ASSETS["startup-about"],
            frozenset({"branding"}),
        )

    def test_tracked_inputs_are_native_configs_with_fixed_capture_geometry(self):
        for engine in ("cpp", "rust"):
            path = REPOSITORY / f"compat/presentation/{engine}.config"
            text = path.read_text(encoding="utf-8")
            self.assertTrue(text.startswith("[General]\n"))
            self.assertIn('LanguageEx="US"\n', text)
            self.assertIn('LanguageCharset=""\n', text)
            self.assertIn('Participants="Presentation.c4p"\n', text)
            self.assertIn("ResolutionX=1280\n", text)
            self.assertIn("ResolutionY=720\n", text)
            self.assertIn("Scale=100\n", text)
            self.assertIn("DisplayMode=Window\n", text)
            self.assertIn("DisableGamma=false\n", text)
            self.assertIn("Shader=true\n", text)
            self.assertEqual(sha256(path), MODULE.NATIVE_CONFIG_SHA256)

        self.assertEqual(
            MODULE.PLAYER_SOURCE_PATH,
            "compat/presentation/player.c4p",
        )
        player = REPOSITORY / MODULE.PLAYER_SOURCE_PATH
        self.assertEqual(sha256(player), MODULE.PLAYER_SHA256)
        self.assertEqual(
            MODULE.PLAYER_SHA256,
            "8dcaf794355d1f8d7e8dfa3efa76b8f601a8a911561d161a3da8ead2a40cd5c0",
        )

        network = REPOSITORY / MODULE.NETWORK_REFERENCES_SOURCE_PATH
        self.assertEqual(sha256(network), MODULE.NETWORK_REFERENCES_SHA256)
        self.assertEqual(
            json.loads(network.read_text(encoding="utf-8")),
            {
                "schema": "clonk-rs/presentation-network-references/v1",
                "references": [],
            },
        )

    def test_tracked_case_specs_pin_the_measured_native_rng_matrix(self):
        specs = MODULE.load_json(REPOSITORY / MODULE.CASE_SPECS_SOURCE_PATH)
        self.assertEqual(
            {
                spec["id"]: spec["port_asset_exemptions"]
                for spec in specs
                if spec["id"] in EXPECTED_LAYOUT_IDS
            },
            EXPECTED_PORT_ASSET_EXEMPTIONS,
        )
        validated = MODULE._validate_v2_case_specs(
            specs,
            expected_case_specs=specs,
            trusted_manifest=MODULE.load_json(
                REPOSITORY / MODULE.CAPTURE_MANIFEST_SOURCE_PATH
            ),
        )
        self.assertEqual([spec["id"] for spec in validated], list(EXPECTED_CASE_IDS))
        by_id = {spec["id"]: spec for spec in validated}
        self.assertEqual(by_id["loader"]["seeds"]["simulation"]["calls"], 0)
        self.assertEqual(
            by_id["loader"]["seeds"]["presentation"],
            {
                "algorithm": MODULE.PRESENTATION_RNG_ALGORITHM,
                "seed": 587,
                "calls": 3,
                "trace_sha256": (
                    "052a4b19a91537982f7319b7bb6cd66ce0c00245d8e583c13b90355c7cff49e0"
                ),
            },
        )
        self.assertEqual(by_id["object-menu"]["seeds"]["simulation"]["calls"], 890)
        self.assertEqual(
            by_id["object-menu"]["seeds"]["presentation"]["trace_sha256"],
            "b6a840dcfa7c6c07c133ce57e8fbf40dcb7780f95335a12b546d535f7b2179a6",
        )
        for spec in validated:
            self.assertEqual(spec["player_sha256"], MODULE.PLAYER_SHA256)
            self.assertEqual(
                spec["runtime_resources"]["cpp"],
                MODULE.PINNED_CPP_RUNTIME_RESOURCES,
            )

    def test_tracked_case_specs_bind_the_current_runtime_resources(self):
        specs = MODULE.load_json(REPOSITORY / MODULE.CASE_SPECS_SOURCE_PATH)
        runtime_resources = MODULE.expected_runtime_resources(
            REPOSITORY,
            REPOSITORY,
            "HEAD",
        )

        for spec in specs:
            with self.subTest(case=spec["id"]):
                self.assertEqual(spec["runtime_resources"], runtime_resources)

    def test_tracked_case_specs_bind_the_pinned_fixture_content_tree(self):
        specs = MODULE.load_json(REPOSITORY / MODULE.CASE_SPECS_SOURCE_PATH)
        fixture_content_tree = MODULE.tree_oid(
            REPOSITORY / "content", MODULE.FIXTURE_CONTENT_COMMIT
        )

        for spec in specs:
            with self.subTest(case=spec["id"]):
                self.assertEqual(spec["scenario"]["content_tree"], fixture_content_tree)

    def test_real_history_keeps_source_gitlink_distinct_from_profile_fixture(self):
        self.assertEqual(
            MODULE.read_gitlink(
                REPOSITORY,
                MODULE.ORACLE_SOURCE_COMMIT,
                "content",
            ),
            MODULE.ORACLE_SOURCE_CONTENT_GITLINK,
        )
        self.assertEqual(
            MODULE.validate_fixture_content_pin(
                REPOSITORY,
                REPOSITORY / "compat/profile.json",
            ),
            MODULE.FIXTURE_CONTENT_COMMIT,
        )

    def test_checkout_head_must_equal_the_requested_oracle_commit(self):
        with tempfile.TemporaryDirectory() as temporary:
            repository = Path(temporary) / "oracle"
            initialise_repository(repository)
            first = commit_file(repository, "source.txt", "one", "first")
            second = commit_file(repository, "source.txt", "two", "second")

            self.assertEqual(
                MODULE.verify_checkout_head(repository, expected_commit=second),
                second,
            )
            with self.assertRaisesRegex(MODULE.AcquisitionFailure, "checkout HEAD"):
                MODULE.verify_checkout_head(repository, expected_commit=first)

    def test_materialization_archives_the_exact_revision_without_mutating_checkout(self):
        with tempfile.TemporaryDirectory() as temporary:
            temporary = Path(temporary)
            repository = temporary / "source"
            initialise_repository(repository)
            first = commit_file(repository, "payload/value.txt", "old", "first")
            commit_file(repository, "payload/value.txt", "working-head", "second")
            destination = temporary / "snapshot"

            MODULE.materialize_git_archive(repository, first, destination)

            self.assertEqual((destination / "payload/value.txt").read_text(), "old")
            self.assertEqual((repository / "payload/value.txt").read_text(), "working-head")
            self.assertEqual(run_git(repository, "status", "--short"), "")
            with self.assertRaisesRegex(MODULE.AcquisitionFailure, "must not exist"):
                MODULE.materialize_git_archive(repository, first, destination)

            inside_checkout = repository / "must-not-be-created"
            with self.assertRaisesRegex(MODULE.AcquisitionFailure, "inside source repository"):
                MODULE.materialize_git_archive(repository, first, inside_checkout)
            self.assertFalse(inside_checkout.exists())
            self.assertEqual(run_git(repository, "status", "--short"), "")

    def test_fixture_content_replaces_an_empty_archive_gitlink_placeholder(self):
        with tempfile.TemporaryDirectory() as temporary:
            temporary = Path(temporary)
            fixture = temporary / "fixture"
            fixture.mkdir()
            (fixture / "Scenario.txt").write_text("fixture", encoding="utf-8")
            destination = temporary / "rust-source/content"
            destination.mkdir(parents=True)

            MODULE._copy_fixture_content(fixture, destination)

            self.assertEqual(
                (destination / "Scenario.txt").read_text(encoding="utf-8"),
                "fixture",
            )

    def test_gitlink_reader_rejects_a_normal_tree_entry(self):
        with tempfile.TemporaryDirectory() as temporary:
            repository = Path(temporary) / "source"
            initialise_repository(repository)
            revision = commit_file(repository, "content/file.txt", "not a submodule", "file")

            with self.assertRaisesRegex(MODULE.AcquisitionFailure, "gitlink"):
                MODULE.read_gitlink(repository, revision, "content")

    def test_profile_pin_must_equal_both_the_expected_pin_and_current_gitlink(self):
        with tempfile.TemporaryDirectory() as temporary:
            temporary = Path(temporary)
            repository = temporary / "workspace"
            initialise_repository(repository)
            commit_file(repository, "README", "workspace", "initial")
            pin = "a" * 40
            install_gitlink(repository, pin)
            profile = repository / "profile.json"
            profile.write_text(
                json.dumps({"pinned": {"content_commit": pin}}),
                encoding="utf-8",
            )

            self.assertEqual(
                MODULE.validate_fixture_content_pin(
                    repository,
                    profile,
                    expected_commit=pin,
                ),
                pin,
            )
            profile.write_text(
                json.dumps({"pinned": {"content_commit": "b" * 40}}),
                encoding="utf-8",
            )
            with self.assertRaisesRegex(MODULE.AcquisitionFailure, "profile content pin"):
                MODULE.validate_fixture_content_pin(
                    repository,
                    profile,
                    expected_commit=pin,
                )

    def test_tree_oids_and_effective_system_scripts_are_content_derived(self):
        profile = MODULE.load_json(REPOSITORY / "compat/profile.json")
        excluded = MODULE.withheld_system_script_paths(profile)
        oracle = MODULE.effective_system_script_manifest(
            REPOSITORY,
            MODULE.ORACLE_SOURCE_COMMIT,
            excluded_paths=(),
        )
        rust = MODULE.effective_system_script_manifest(
            REPOSITORY,
            "HEAD",
            excluded_paths=excluded,
        )

        self.assertEqual(oracle, rust)
        self.assertEqual(len(oracle["sha256"]), 64)
        self.assertTrue(oracle["entries"])
        self.assertEqual(
            MODULE.tree_oid(REPOSITORY / "content", MODULE.FIXTURE_CONTENT_COMMIT),
            "092a7dd9a43f0d87e8d6dd9957325c44668776b9",
        )

    def test_provenance_can_revalidate_the_recorded_pre_evidence_rust_commit(self):
        with tempfile.TemporaryDirectory() as temporary:
            oracle = Path(temporary) / "oracle"
            subprocess.run(
                ["git", "clone", "--quiet", "--shared", "--no-checkout", str(REPOSITORY), str(oracle)],
                check=True,
            )
            run_git(oracle, "checkout", "--quiet", MODULE.ORACLE_SOURCE_COMMIT)
            rust_revision = run_git(REPOSITORY, "rev-parse", "HEAD^")

            fields = MODULE.expected_provenance_fields(
                oracle, rust_revision=rust_revision
            )

            self.assertEqual(fields["rust_source_commit"], rust_revision)
            self.assertEqual(
                fields["rust_source_tree"],
                run_git(REPOSITORY, "rev-parse", f"{rust_revision}^{{tree}}"),
            )


class InventoryAndPngTests(unittest.TestCase):
    def test_repository_contract_holds_accepted_presentation_evidence(self):
        manifest = MODULE.load_json(REPOSITORY / MODULE.CAPTURE_MANIFEST_SOURCE_PATH)
        profile = MODULE.load_json(REPOSITORY / "compat/profile.json")

        MODULE._validate_final_presentation_lifecycle(manifest, profile)

    def test_case_inventory_requires_all_thirteen_and_exactly_eleven_layout_cases(self):
        MODULE.validate_case_inventory(EXPECTED_CASE_IDS, EXPECTED_LAYOUT_IDS)
        for capture_ids, layout_ids in (
            (EXPECTED_CASE_IDS[:-1], EXPECTED_LAYOUT_IDS),
            ((*EXPECTED_CASE_IDS, "extra"), EXPECTED_LAYOUT_IDS),
            (EXPECTED_CASE_IDS, frozenset((*EXPECTED_LAYOUT_IDS, "loader"))),
            (EXPECTED_CASE_IDS, frozenset(set(EXPECTED_LAYOUT_IDS) - {"startup-main"})),
        ):
            with self.subTest(capture_ids=capture_ids, layout_ids=layout_ids):
                with self.assertRaises(MODULE.AcquisitionFailure):
                    MODULE.validate_case_inventory(capture_ids, layout_ids)

    def test_png_validator_accepts_only_1280x720_eight_bit_rgb_or_rgba(self):
        with tempfile.TemporaryDirectory() as temporary:
            temporary = Path(temporary)
            for color_type in (2, 6):
                image = temporary / f"valid-{color_type}.png"
                image.write_bytes(png_bytes(color_type=color_type))
                self.assertEqual(
                    MODULE.validate_png(image),
                    {"width": 1280, "height": 720, "bit_depth": 8, "color_type": color_type},
                )

            invalid = {
                "signature": b"not-png",
                "width": png_bytes(width=1279),
                "height": png_bytes(height=719),
                "depth": png_bytes(bit_depth=16),
                "indexed": png_bytes(color_type=3),
                "interlaced": png_bytes(interlace=1),
            }
            for label, contents in invalid.items():
                with self.subTest(label=label):
                    image = temporary / f"invalid-{label}.png"
                    image.write_bytes(contents)
                    with self.assertRaises(MODULE.AcquisitionFailure):
                        MODULE.validate_png(image)

    def test_png_validator_checks_the_ihdr_crc(self):
        with tempfile.TemporaryDirectory() as temporary:
            image = Path(temporary) / "corrupt.png"
            contents = bytearray(png_bytes())
            contents[29] ^= 1
            image.write_bytes(contents)
            with self.assertRaisesRegex(MODULE.AcquisitionFailure, "IHDR CRC"):
                MODULE.validate_png(image)

    def test_png_validator_requires_a_complete_decodable_scanline_stream(self):
        with tempfile.TemporaryDirectory() as temporary:
            temporary = Path(temporary)
            valid = png_bytes()
            ihdr_end = 8 + 12 + 13
            without_idat = valid[:ihdr_end] + png_chunk(b"IEND", b"")
            first_idat_crc = ihdr_end + 4 + 4 + struct.unpack(
                ">I", valid[ihdr_end : ihdr_end + 4]
            )[0]
            corrupt_idat_crc = bytearray(valid)
            corrupt_idat_crc[first_idat_crc] ^= 1
            invalid = {
                "missing-idat": without_idat,
                "truncated-chunk": valid[:-3],
                "trailing-bytes": valid + b"not-part-of-the-png",
                "idat-crc": bytes(corrupt_idat_crc),
                "invalid-zlib": png_bytes(compressed_payload=b"not-zlib"),
                "short-scanlines": png_bytes(raw_adjustment=-1),
                "long-scanlines": png_bytes(raw_adjustment=1),
                "illegal-filter": png_bytes(illegal_filter_row=719),
            }
            for label, contents in invalid.items():
                with self.subTest(label=label):
                    image = temporary / f"invalid-{label}.png"
                    image.write_bytes(contents)
                    with self.assertRaises(MODULE.AcquisitionFailure):
                        MODULE.validate_png(image)

            split_idat = temporary / "valid-split-idat.png"
            split_idat.write_bytes(png_bytes(color_type=6, idat_parts=3))
            self.assertEqual(MODULE.validate_png(split_idat)["color_type"], 6)

    def test_duplicate_runs_require_every_explicit_artifact_to_be_byte_identical(self):
        with tempfile.TemporaryDirectory() as temporary:
            temporary = Path(temporary)
            first = temporary / "oracle"
            second = temporary / "repeat"
            write_capture_set(first)
            write_capture_set(second)

            artifacts = MODULE.validate_duplicate_runs(first, second)
            self.assertEqual(len(artifacts), 24)

            (second / "gameplay.png").write_bytes(png_bytes(sample_byte=1))
            with self.assertRaisesRegex(MODULE.AcquisitionFailure, "gameplay.png"):
                MODULE.validate_duplicate_runs(first, second)

    def test_duplicate_runs_reject_missing_and_symlinked_artifacts(self):
        with tempfile.TemporaryDirectory() as temporary:
            temporary = Path(temporary)
            first = temporary / "oracle"
            second = temporary / "repeat"
            write_capture_set(first)
            write_capture_set(second)
            (second / "loader.png").unlink()
            with self.assertRaisesRegex(MODULE.AcquisitionFailure, "loader.png"):
                MODULE.validate_duplicate_runs(first, second)

            (second / "loader.png").symlink_to(first / "loader.png")
            with self.assertRaisesRegex(MODULE.AcquisitionFailure, "regular file"):
                MODULE.validate_duplicate_runs(first, second)

    def test_layout_trace_matches_the_strict_ordered_comparator_contract(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "oracle"
            write_capture_set(root)
            path = root / "startup-main.layout.json"
            valid_trace = json.loads(path.read_text(encoding="utf-8"))

            MODULE.validate_layout_trace(path, "startup-main")
            trace = copy.deepcopy(valid_trace)
            tagged = next(
                element for element in trace["elements"] if "port_asset" in element
            )
            tagged["path"] = "startup/main/buttons/start-game"
            path.write_text(json.dumps(trace), encoding="utf-8")
            with self.assertRaisesRegex(
                MODULE.AcquisitionFailure, "port asset exemption path"
            ):
                MODULE.validate_layout_trace(path, "startup-main")

            mutations = {
                "top-level key": lambda value: value.update(extra=True),
                "manifest resolution": lambda value: value.update(resolution=[1280, 720]),
                "scale": lambda value: value.update(scale=1),
                "screen": lambda value: value.update(screen="startup-options"),
                "element key": lambda value: value["elements"][0].update(extra=True),
                "role type": lambda value: value["elements"][0].update(role=1),
                "rect key": lambda value: value["elements"][0]["rect"].pop("width"),
                "visible type": lambda value: value["elements"][0].update(visible=1),
                "port asset type": lambda value: value["elements"][0].update(
                    port_asset=1
                ),
                "line key": lambda value: value["elements"][0]["lines"][0].update(
                    extra=True
                ),
                "line text type": lambda value: value["elements"][0]["lines"][0].update(
                    text=1
                ),
            }
            for label, mutate in mutations.items():
                with self.subTest(label=label):
                    invalid = copy.deepcopy(valid_trace)
                    mutate(invalid)
                    path.write_text(json.dumps(invalid), encoding="utf-8")
                    with self.assertRaises(MODULE.AcquisitionFailure):
                        MODULE.validate_layout_trace(path, "startup-main")

    def test_layout_trace_rejects_an_empty_element_path(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "oracle"
            write_capture_set(root)
            path = root / "startup-main.layout.json"
            trace = json.loads(path.read_text(encoding="utf-8"))
            trace["elements"][0]["path"] = "  "
            path.write_text(json.dumps(trace), encoding="utf-8")

            with self.assertRaisesRegex(MODULE.AcquisitionFailure, "path must be non-empty"):
                MODULE.validate_layout_trace(path, "startup-main")

    def test_layout_trace_rejects_an_empty_semantic_role(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "oracle"
            write_capture_set(root)
            path = root / "startup-main.layout.json"
            trace = json.loads(path.read_text(encoding="utf-8"))
            trace["elements"][0]["role"] = ""
            path.write_text(json.dumps(trace), encoding="utf-8")

            with self.assertRaisesRegex(MODULE.AcquisitionFailure, "role must be non-empty"):
                MODULE.validate_layout_trace(path, "startup-main")

    def test_layout_trace_rejects_duplicate_element_paths(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "oracle"
            write_capture_set(root)
            path = root / "startup-main.layout.json"
            trace = json.loads(path.read_text(encoding="utf-8"))
            trace["elements"].append(copy.deepcopy(trace["elements"][0]))
            path.write_text(json.dumps(trace), encoding="utf-8")

            with self.assertRaisesRegex(MODULE.AcquisitionFailure, "duplicate element path"):
                MODULE.validate_layout_trace(path, "startup-main")

    def test_layout_trace_rejects_an_empty_element_inventory(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "oracle"
            write_capture_set(root)
            path = root / "startup-main.layout.json"
            trace = json.loads(path.read_text(encoding="utf-8"))
            trace["elements"] = []
            path.write_text(json.dumps(trace), encoding="utf-8")

            with self.assertRaisesRegex(MODULE.AcquisitionFailure, "must not be empty"):
                MODULE.validate_layout_trace(path, "startup-main")


class AcquisitionOrchestrationTests(unittest.TestCase):
    def test_acquire_threads_staged_cpp_runtime_content_into_capture(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            workspace = root / "workspace"
            oracle = root / "oracle"
            candidate = root / "candidate"
            workspace.mkdir()
            oracle.mkdir()
            resources = synthetic_runtime_resources()
            context = {
                "runtime_resources": resources,
                "fixture_content_tree": "c" * 40,
                "rust_source_commit": "a" * 40,
                "rust_source_tree": "b" * 40,
            }
            runtime_content = {"tutorial": {"tree": "d" * 40}}
            staged = {
                "cpp_runtime_content_resources": runtime_content,
                "cpp_runtime_scenario_resources": {"scenario": "resources"},
                "oracle_source": root / "oracle-source",
                "rust_source": root / "rust-source",
            }
            binaries = {"cpp": root / "cpp", "rust": root / "rust"}

            with (
                mock.patch.object(MODULE, "verify_checkout_head"),
                mock.patch.object(
                    MODULE,
                    "require_clean_source_revision",
                    return_value=("a" * 40, "b" * 40),
                ),
                mock.patch.object(
                    MODULE,
                    "_require_fresh_external_output",
                    return_value=candidate,
                ),
                mock.patch.object(
                    MODULE, "expected_provenance_fields", return_value=context
                ),
                mock.patch.object(
                    MODULE,
                    "load_json_at_revision",
                    side_effect=[{}, {}, []],
                ),
                mock.patch.object(MODULE, "_validate_v2_case_specs", return_value=[]),
                mock.patch.object(
                    MODULE, "rust_source_input_inventory", return_value={}
                ),
                mock.patch.object(
                    MODULE, "_stage_acquisition", return_value=staged
                ) as stage,
                mock.patch.object(MODULE, "_acquisition_inputs", return_value={}),
                mock.patch.object(MODULE, "_validate_case_input_hashes"),
                mock.patch.object(MODULE, "_write_json_new"),
                mock.patch.object(MODULE, "_sha256_file", return_value="e" * 64),
                mock.patch.object(
                    MODULE,
                    "_build_acquisition_binaries",
                    return_value=({}, binaries),
                ),
                mock.patch.object(
                    MODULE,
                    "_run_acquisition_captures",
                    side_effect=MODULE.AcquisitionFailure("capture sentinel"),
                ) as capture,
            ):
                with self.assertRaisesRegex(
                    MODULE.AcquisitionFailure, "capture sentinel"
                ):
                    MODULE.acquire_presentation_oracle(
                        oracle,
                        candidate,
                        workspace=workspace,
                    )

            self.assertNotIn("runtime_content_resources", stage.call_args.kwargs)
            self.assertIs(
                capture.call_args.kwargs["runtime_content_resources"],
                runtime_content,
            )
            self.assertIs(
                capture.call_args.kwargs["scenario_resources"],
                staged["cpp_runtime_scenario_resources"],
            )

    def test_cpp_runtime_cleanup_removes_only_bound_acquisition_resources_and_log(self):
        with tempfile.TemporaryDirectory() as temporary:
            candidate = Path(temporary) / "candidate"
            inputs = candidate / "inputs"
            runtime_resources = {}
            for group, (_, retained_path) in MODULE.RUNTIME_RESOURCE_GROUPS.items():
                directory = candidate / retained_path
                directory.mkdir(parents=True)
                (directory / "resource.bin").write_bytes(group.encode("ascii"))
                runtime_resources[group] = MODULE.runtime_resource_identity(directory)
            runtime_content = {}
            for group, (_, retained_path) in MODULE.CPP_RUNTIME_CONTENT_GROUPS.items():
                directory = candidate / retained_path
                directory.mkdir(parents=True)
                (directory / "content.bin").write_bytes(group.encode("ascii"))
                runtime_content[group] = MODULE.runtime_resource_identity(directory)
            (inputs / "Clonk.log").write_text("capture log\n", encoding="utf-8")
            retained = inputs / "cpp.config"
            retained.write_text("retained\n", encoding="utf-8")

            MODULE._remove_cpp_runtime_resources(
                candidate,
                runtime_resources,
                runtime_content,
            )

            self.assertTrue(retained.is_file())
            self.assertFalse((inputs / "Clonk.log").exists())
            for _, retained_path in (
                *MODULE.RUNTIME_RESOURCE_GROUPS.values(),
                *MODULE.CPP_RUNTIME_CONTENT_GROUPS.values(),
            ):
                self.assertFalse((candidate / retained_path).exists())

    def test_rust_capture_rejects_archive_filtered_runtime_resources(self):
        with tempfile.TemporaryDirectory() as temporary:
            candidate = Path(temporary) / "candidate"
            rust_source = candidate / "work/rust-source"
            expected_root = candidate / "expected"
            expected = {}
            for group, (source_path, _) in MODULE.RUNTIME_RESOURCE_GROUPS.items():
                raw = expected_root / source_path
                staged = rust_source / source_path
                raw.mkdir(parents=True)
                staged.mkdir(parents=True)
                (raw / "resource.txt").write_bytes(b"one\ntwo\n")
                (staged / "resource.txt").write_bytes(b"one\r\ntwo\r\n")
                expected[group] = MODULE.runtime_resource_identity(raw)

            with self.assertRaisesRegex(
                MODULE.AcquisitionFailure, "Rust.*runtime resource.*mismatch"
            ):
                MODULE._capture_command(
                    candidate,
                    {"rust": rust_source / "target/release/clonk-app"},
                    engine="rust",
                    case_id="gameplay",
                    runtime_resources=expected,
                    rust_source_root=rust_source,
                )

    def test_cpp_network_cache_is_fresh_per_lobby_run_and_removed_safely(self):
        with tempfile.TemporaryDirectory() as temporary:
            candidate = Path(temporary) / "candidate"
            network = candidate / "inputs/Network"
            network.mkdir(parents=True)
            (network / "Reference.tmp").write_bytes(b"acquisition-only")
            with self.assertRaisesRegex(
                MODULE.AcquisitionFailure, "Network.*not fresh"
            ):
                MODULE._require_fresh_cpp_network_cache(candidate)

            MODULE._remove_cpp_network_cache(candidate)
            self.assertFalse(network.exists())
            MODULE._require_fresh_cpp_network_cache(candidate)

            outside = candidate / "outside"
            outside.mkdir()
            network.symlink_to(outside, target_is_directory=True)
            with self.assertRaisesRegex(
                MODULE.AcquisitionFailure, "symlink or special"
            ):
                MODULE._remove_cpp_network_cache(candidate)
            self.assertTrue(outside.is_dir())

    def test_every_cpp_launch_rejects_a_preexisting_network_cache(self):
        with tempfile.TemporaryDirectory() as temporary:
            candidate = Path(temporary) / "candidate"
            network = candidate / "inputs/Network"
            network.mkdir(parents=True)
            (network / "stale-reference").write_bytes(b"stale")
            runtime_resources = synthetic_runtime_resources()

            with (
                mock.patch.object(
                    MODULE,
                    "_capture_command",
                    return_value=([str(candidate / "clonk")], candidate / "inputs"),
                ),
                mock.patch.object(MODULE, "_run") as run,
            ):
                with self.assertRaisesRegex(
                    MODULE.AcquisitionFailure, "Network.*not fresh"
                ):
                    MODULE._run_capture_case(
                        candidate,
                        {"cpp": candidate / "clonk"},
                        run_id="run-1",
                        nonce="a" * 64,
                        engine="cpp",
                        case_id="startup-main",
                        runtime_resources=runtime_resources,
                    )
            run.assert_not_called()

    def test_each_headed_capture_uses_the_short_capture_timeout(self):
        with tempfile.TemporaryDirectory() as temporary:
            candidate = Path(temporary) / "candidate"
            command = [str(candidate / "clonk-app")]
            with (
                mock.patch.object(
                    MODULE,
                    "_capture_command",
                    return_value=(command, candidate),
                ),
                mock.patch.object(MODULE, "_prepare_capture_user_data_directory"),
                mock.patch.object(
                    MODULE,
                    "_run",
                    side_effect=MODULE.AcquisitionFailure("stop after launch"),
                ) as run,
            ):
                with self.assertRaisesRegex(
                    MODULE.AcquisitionFailure, "stop after launch"
                ):
                    MODULE._run_capture_case(
                        candidate,
                        {"rust": candidate / "clonk-app"},
                        run_id="run-1",
                        nonce="a" * 64,
                        engine="rust",
                        case_id="gameplay",
                        runtime_resources=synthetic_runtime_resources(),
                    )

            self.assertEqual(
                run.call_args.kwargs["timeout_seconds"],
                MODULE.CAPTURE_COMMAND_TIMEOUT_SECONDS,
            )

    def test_capture_runtime_environment_strips_hostile_project_and_locale_state(self):
        hostile = {
            "PATH": "/audited/bin",
            "HOME": "/audited/home",
            "DYLD_LIBRARY_PATH": "/audited/fmt",
            "LD_LIBRARY_PATH": "/audited/injected-libs",
            "LC_CTYPE": "host-locale",
            "LC_CONTENT_DIR": "/host/content",
            "LC_USER_DATA_DIR": "/host/user-data",
            "LC_RUST_ENGINE_RUNTIME": "1",
            "C4DEBUGREC": "1",
            "CLONK_PRESENTATION_CASE": "stale-case",
            "CLONK_PROFILE": "noncanonical",
            "SDL_RENDER_DRIVER": "host-selected",
            "MESA_LOADER_DRIVER_OVERRIDE": "host-selected",
            "UNRELATED_SECRET": "must-not-leak",
        }
        overrides = {
            "CLONK_PRESENTATION_CASE": "gameplay",
            "LANG": "C",
            "LC_ALL": "C",
            "TZ": "UTC",
        }
        with mock.patch.dict(MODULE.os.environ, hostile, clear=True):
            environment = MODULE._capture_runtime_environment(overrides)

        self.assertEqual(
            environment,
            {
                "PATH": "/audited/bin",
                **overrides,
            },
        )

    def test_capture_audio_driver_is_pinned_to_dummy_for_both_engines(self):
        hostile = {
            "PATH": "/audited/bin",
            "SDL_AUDIODRIVER": "coreaudio",
        }
        with tempfile.TemporaryDirectory() as temporary:
            candidate = Path(temporary) / "candidate"
            candidate.mkdir()
            for engine in MODULE.ENGINE_IDS:
                with self.subTest(engine=engine):
                    with mock.patch.dict(MODULE.os.environ, hostile, clear=True):
                        controlled = MODULE._capture_environment(
                            candidate,
                            run_id="run-1",
                            nonce="a" * 64,
                            engine=engine,
                            case_id="object-menu",
                        )
                        environment = MODULE._capture_runtime_environment(controlled)

                    self.assertEqual(controlled["SDL_AUDIODRIVER"], "dummy")
                    self.assertEqual(environment["SDL_AUDIODRIVER"], "dummy")
                    self.assertNotIn("coreaudio", environment.values())

    def test_capture_user_data_is_candidate_local_and_must_be_fresh(self):
        with tempfile.TemporaryDirectory() as temporary:
            candidate = Path(temporary) / "candidate"
            candidate.mkdir()
            environment = MODULE._capture_environment(
                candidate,
                run_id="run-1",
                nonce="a" * 64,
                engine="rust",
                case_id="gameplay",
            )
            user_data = candidate / "work/user-data/run-1/rust/gameplay"
            self.assertEqual(environment["LC_USER_DATA_DIR"], str(user_data))

            MODULE._prepare_capture_user_data_directory(user_data)
            self.assertEqual(list(user_data.iterdir()), [])
            (user_data / "Scenario.txt").write_text("host injection", encoding="utf-8")
            with self.assertRaisesRegex(
                MODULE.AcquisitionFailure, "user-data directory is not fresh"
            ):
                MODULE._prepare_capture_user_data_directory(user_data)

    def test_rust_player_discovery_uses_only_the_hash_bound_canonical_player(self):
        with tempfile.TemporaryDirectory() as temporary:
            candidate = Path(temporary) / "candidate"
            retained = candidate / MODULE.PLAYER_RETAINED_PATH
            retained.parent.mkdir(parents=True)
            retained.write_bytes((REPOSITORY / MODULE.PLAYER_SOURCE_PATH).read_bytes())
            rust_source = candidate / "work/rust-source"
            rust_source.mkdir(parents=True)

            installed = MODULE._install_rust_player_discovery_fixture(
                candidate, rust_source
            )
            self.assertEqual(installed, rust_source / "Presentation.c4p")
            self.assertEqual(sha256(installed), MODULE.PLAYER_SHA256)
            with self.assertRaisesRegex(
                MODULE.AcquisitionFailure, "player discovery fixture.*already exists"
            ):
                MODULE._install_rust_player_discovery_fixture(candidate, rust_source)

            MODULE._remove_rust_player_discovery_fixture(
                candidate, rust_source
            )
            self.assertFalse(installed.exists())

    def test_build_environment_strips_unrecorded_compiler_and_cargo_overrides(self):
        hostile = {
            "PATH": "/audited/bin",
            "HOME": "/audited/home",
            "RUSTUP_HOME": "/audited/rustup",
            "CARGO_HOME": "/audited/cargo",
            "TMPDIR": "/audited/tmp",
            "RUSTFLAGS": "-C target-cpu=native",
            "CARGO_TARGET_DIR": "/substituted-target",
            "RUSTC_WRAPPER": "/substituted-wrapper",
            "CC": "/substituted-cc",
            "CXX": "/substituted-cxx",
            "CFLAGS": "-march=native",
            "CXXFLAGS": "-march=native",
            "LDFLAGS": "-L/substituted",
            "DYLD_INSERT_LIBRARIES": "/substituted.dylib",
            "CLONK_PRESENTATION_CASE": "stale-case",
        }
        with mock.patch.dict(MODULE.os.environ, hostile, clear=True):
            environment = MODULE._build_process_environment(
                MODULE.BUILD_ENVIRONMENT
            )

        self.assertEqual(
            environment,
            {
                "PATH": "/audited/bin",
                "HOME": "/audited/home",
                "RUSTUP_HOME": "/audited/rustup",
                "CARGO_HOME": "/audited/cargo",
                "TMPDIR": "/audited/tmp",
                **MODULE.BUILD_ENVIRONMENT,
            },
        )
        self.assertEqual(environment["CARGO_INCREMENTAL"], "0")

    def test_cpp_startup_launch_requires_exact_runtime_resource_trees(self):
        with tempfile.TemporaryDirectory() as temporary:
            candidate = Path(temporary) / "candidate"
            graphics = candidate / "inputs/Graphics.c4g"
            system = candidate / "inputs/System.c4g"
            graphics.mkdir(parents=True)
            system.mkdir()
            (graphics / "Logo.png").write_bytes(b"pinned graphics")
            (system / "LanguageUS.txt").write_bytes(b"pinned system")
            run_git(candidate, "init", "--quiet")
            run_git(candidate, "add", "inputs/Graphics.c4g", "inputs/System.c4g")
            root_tree = run_git(candidate, "write-tree")
            runtime_resources = {
                "graphics": {
                    "tree": run_git(
                        candidate,
                        "rev-parse",
                        f"{root_tree}:inputs/Graphics.c4g",
                    ),
                    "manifest_sha256": resource_manifest_sha256(
                        {"Logo.png": b"pinned graphics"}
                    ),
                },
                "system": {
                    "tree": run_git(
                        candidate,
                        "rev-parse",
                        f"{root_tree}:inputs/System.c4g",
                    ),
                    "manifest_sha256": resource_manifest_sha256(
                        {"LanguageUS.txt": b"pinned system"}
                    ),
                },
            }
            binaries = {"cpp": candidate / "clonk"}

            command, cwd = MODULE._capture_command(
                candidate,
                binaries,
                engine="cpp",
                case_id="startup-main",
                runtime_resources=runtime_resources,
            )
            self.assertEqual(cwd, candidate / "inputs")
            self.assertEqual(command[1], "/startup:main")

            (graphics / "Logo.png").write_bytes(b"substituted graphics")
            with self.assertRaisesRegex(MODULE.AcquisitionFailure, "Graphics.c4g.*digest"):
                MODULE._capture_command(
                    candidate,
                    binaries,
                    engine="cpp",
                    case_id="startup-main",
                    runtime_resources=runtime_resources,
                )
            (graphics / "Logo.png").write_bytes(b"pinned graphics")
            (system / "LanguageUS.txt").unlink()
            with self.assertRaisesRegex(MODULE.AcquisitionFailure, "System.c4g"):
                MODULE._capture_command(
                    candidate,
                    binaries,
                    engine="cpp",
                    case_id="startup-main",
                    runtime_resources=runtime_resources,
                )

    def test_cpp_runtime_launch_rehashes_the_selected_scenario_subtree(self):
        with tempfile.TemporaryDirectory() as temporary:
            candidate = Path(temporary) / "candidate"
            scenario_resources = {}
            for scenario_path in MODULE.CPP_RUNTIME_SCENARIO_PATHS:
                scenario = candidate / "work/fixture-content" / scenario_path
                scenario.mkdir(parents=True)
                (scenario / "Scenario.txt").write_text(
                    f"{scenario_path}\n",
                    encoding="utf-8",
                )
                scenario_resources[scenario_path] = MODULE.runtime_resource_identity(
                    scenario
                )
            gameplay = (
                candidate
                / "work/fixture-content"
                / MODULE.CPP_RUNTIME_SCENARIOS["gameplay"]
            )
            (gameplay / "Scenario.txt").write_text("tampered\n", encoding="utf-8")

            with (
                mock.patch.object(MODULE, "_validate_staged_cpp_runtime_resources"),
                mock.patch.object(MODULE, "_validate_staged_cpp_runtime_content"),
            ):
                with self.assertRaisesRegex(
                    MODULE.AcquisitionFailure, "gameplay scenario.*mismatch"
                ):
                    MODULE._capture_command(
                        candidate,
                        {"cpp": candidate / "clonk"},
                        engine="cpp",
                        case_id="gameplay",
                        runtime_resources=synthetic_runtime_resources()["cpp"],
                        runtime_content_resources={
                            group: {"tree": "a" * 40, "manifest_sha256": "b" * 64}
                            for group in MODULE.CPP_RUNTIME_CONTENT_GROUPS
                        },
                        scenario_resources=scenario_resources,
                    )

    def test_cpp_runtime_launch_accepts_a_hash_bound_directory_group(self):
        with tempfile.TemporaryDirectory() as temporary:
            candidate = Path(temporary) / "candidate"
            scenario_resources = {}
            for scenario_path in MODULE.CPP_RUNTIME_SCENARIO_PATHS:
                scenario = candidate / "work/fixture-content" / scenario_path
                scenario.mkdir(parents=True)
                (scenario / "Scenario.txt").write_text(
                    f"{scenario_path}\n",
                    encoding="utf-8",
                )
                scenario_resources[scenario_path] = MODULE.runtime_resource_identity(
                    scenario
                )

            with (
                mock.patch.object(MODULE, "_validate_staged_cpp_runtime_resources"),
                mock.patch.object(MODULE, "_validate_staged_cpp_runtime_content"),
            ):
                command, cwd = MODULE._capture_command(
                    candidate,
                    {"cpp": candidate / "clonk"},
                    engine="cpp",
                    case_id="gameplay",
                    runtime_resources=synthetic_runtime_resources()["cpp"],
                    runtime_content_resources={
                        group: {"tree": "a" * 40, "manifest_sha256": "b" * 64}
                        for group in MODULE.CPP_RUNTIME_CONTENT_GROUPS
                    },
                    scenario_resources=scenario_resources,
                )

            self.assertEqual(
                command[1],
                str(
                    candidate
                    / "work/fixture-content"
                    / MODULE.CPP_RUNTIME_SCENARIOS["gameplay"]
                ),
            )
            self.assertEqual(cwd, candidate / "inputs")

    def test_build_recipes_record_every_executed_command_without_a_shell(self):
        cpp = MODULE.cpp_build_recipe()
        rust = MODULE.rust_build_recipe()
        self.assertEqual(len(cpp["commands"]), 2)
        self.assertEqual(
            cpp["commands"][0]["argv"][:7],
            ["cmake", "-S", ".", "-B", "build", "-G", "Ninja"],
        )
        self.assertEqual(
            cpp["commands"][1]["argv"],
            ["cmake", "--build", "build", "--target", "clonk", "-j", "4"],
        )
        self.assertEqual(
            rust["commands"][0]["argv"],
            [
                "cargo",
                "build",
                "--locked",
                "--release",
                "-p",
                "clonk-app",
                "--features",
                "presentation-capture",
            ],
        )
        for recipe in (cpp, rust):
            self.assertEqual(
                recipe["sha256"],
                canonical_sha256({"commands": recipe["commands"]}),
            )

    def test_audited_build_streams_its_output(self):
        repository = Path("/checkout")
        recipe = MODULE.rust_current_build_recipe(repository, "test")
        with mock.patch.object(MODULE, "_run") as run:
            MODULE._execute_current_rust_build(
                recipe,
                repository,
                build_profile="test",
            )

        self.assertFalse(run.call_args.kwargs["capture_output"])

    def test_cpp_capture_patch_links_the_pinned_fmt_headers_without_a_runtime_dylib(self):
        patch = (REPOSITORY / MODULE.CAPTURE_PATCH_SOURCE_PATH).read_text(
            encoding="utf-8"
        )

        self.assertIn(
            "-target_link_libraries(standard fmt::fmt)\n"
            "+target_link_libraries(standard fmt::fmt-header-only)\n",
            patch,
        )

    def test_launch_contract_uses_exact_inputs_and_refuses_unaudited_cpp_runtime(self):
        candidate = Path("/tmp/presentation-candidate")
        binaries = {
            "cpp": candidate
            / "work/oracle-source/build/clonk.app/Contents/MacOS/clonk",
            "rust": candidate / "work/rust-source/target/release/clonk-app",
        }
        expected_startup_arguments = {
            "startup-main": "/startup:main",
            "startup-scenario-selection": "/startup:scen",
            "startup-network-browser": "/startup:net",
            "startup-player-selection": "/startup:plrsel",
            "startup-options": "/startup:options",
            "startup-about": "/startup:about",
        }
        self.assertEqual(MODULE.CPP_STARTUP_ARGUMENTS, expected_startup_arguments)
        self.assertEqual(tuple(MODULE.CPP_STARTUP_ARGUMENTS), EXPECTED_CASE_IDS[:6])
        with mock.patch.object(MODULE, "_validate_staged_cpp_runtime_resources"):
            for case_id, startup_argument in expected_startup_arguments.items():
                with self.subTest(case_id=case_id):
                    cpp, cpp_cwd = MODULE._capture_command(
                        candidate,
                        binaries,
                        engine="cpp",
                        case_id=case_id,
                        runtime_resources=synthetic_runtime_resources()["cpp"],
                    )
                    self.assertEqual(cpp_cwd, candidate / "inputs")
                    self.assertEqual(
                        cpp,
                        [
                            str(binaries["cpp"]),
                            startup_argument,
                            f"/config:{candidate / 'inputs/cpp.config'}",
                        ],
                    )
        with (
            mock.patch.object(MODULE, "_validate_staged_rust_runtime_resources"),
            mock.patch.object(
                MODULE, "_validate_staged_cpp_runtime_content"
            ) as validate_content,
        ):
            rust, rust_cwd = MODULE._capture_command(
                candidate,
                binaries,
                engine="rust",
                case_id="gameplay",
                runtime_resources=synthetic_runtime_resources()["rust"],
                runtime_content_resources={
                    group: {"tree": "a" * 40, "manifest_sha256": "b" * 64}
                    for group in MODULE.CPP_RUNTIME_CONTENT_GROUPS
                },
            )
        validate_content.assert_called_once()
        self.assertEqual(rust_cwd, candidate / "work/rust-source")
        self.assertEqual(rust[-2], str(candidate / MODULE.PLAYER_RETAINED_PATH))
        environment = MODULE._capture_environment(
            candidate,
            run_id="run-1",
            nonce="a" * 64,
            engine="rust",
            case_id="gameplay",
        )
        self.assertEqual(
            environment["CLONK_PRESENTATION_SOURCE_IDENTITY"],
            str(candidate / MODULE.RUST_SOURCE_IDENTITY_RETAINED_PATH),
        )
        self.assertEqual(environment["LC_CONTENT_DIR"], str(candidate / "inputs"))
        self.assertNotIn("CLONK_PRESENTATION_NETWORK_REFERENCES", environment)
        runtime_scenarios = {
            "network-lobby": "Tutorial01.c4s",
            "loader": "Tutorial01.c4s",
            "hud": "Tutorial01.c4s",
            "ingame-menu": "Tutorial01.c4s",
            "object-menu": "Tutorial03.c4s",
            "gameplay": "Tutorial02.c4s",
            "evaluation": "Tutorial01.c4s",
        }
        with (
            mock.patch.object(MODULE, "_validate_staged_cpp_runtime_resources"),
            mock.patch.object(MODULE, "_validate_staged_cpp_runtime_content"),
            mock.patch.object(MODULE, "_validate_staged_cpp_runtime_scenario"),
            mock.patch.object(MODULE, "_regular_file"),
        ):
            for case_id, scenario_name in runtime_scenarios.items():
                with self.subTest(case_id=case_id):
                    command, cwd = MODULE._capture_command(
                        candidate,
                        binaries,
                        engine="cpp",
                        case_id=case_id,
                        runtime_resources=synthetic_runtime_resources()["cpp"],
                        runtime_content_resources={
                            group: {"tree": "a" * 40, "manifest_sha256": "b" * 64}
                            for group in MODULE.CPP_RUNTIME_CONTENT_GROUPS
                        },
                        scenario_resources={
                            path: {"tree": "a" * 40, "manifest_sha256": "b" * 64}
                            for path in MODULE.CPP_RUNTIME_SCENARIO_PATHS
                        },
                    )
                    scenario = (
                        candidate
                        / "work/fixture-content/Tutorial.c4f"
                        / scenario_name
                    )
                    flags = ["/network", "/lobby"] if case_id == "network-lobby" else []
                    self.assertEqual(
                        command,
                        [
                            str(binaries["cpp"]),
                            str(scenario),
                            *flags,
                            f"/config:{candidate / 'inputs/cpp.config'}",
                        ],
                    )
                    self.assertEqual(cwd, candidate / "inputs")

    def test_rust_capture_binary_stays_beneath_the_staged_install_root(self):
        candidate = Path("/tmp/presentation-candidate")
        rust_source = candidate / "work/rust-source"
        with (
            mock.patch.object(MODULE, "_validate_staged_rust_runtime_resources"),
            mock.patch.object(MODULE, "_validate_staged_cpp_runtime_content"),
        ):
            with self.assertRaisesRegex(
                MODULE.AcquisitionFailure,
                "Rust capture executable.*staged install root",
            ):
                MODULE._capture_command(
                    candidate,
                    {"rust": candidate / "builds/rust/binary"},
                    engine="rust",
                    case_id="gameplay",
                    runtime_resources=synthetic_runtime_resources()["rust"],
                    runtime_content_resources={
                        group: {"tree": "a" * 40, "manifest_sha256": "b" * 64}
                        for group in MODULE.CPP_RUNTIME_CONTENT_GROUPS
                    },
                    rust_source_root=rust_source,
                )

    def test_live_comparator_is_no_args_and_requires_the_exact_success_receipt(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            binary = root / "clonk-app"
            reference = root / "reference.png"
            actual = root / "actual.png"
            for path in (binary, reference, actual):
                path.write_bytes(b"regular")
            success = {
                "schema": "clonk-rs/presentation-comparison/v1",
                "case_id": "loader",
                "comparison": "pixel",
                "status": "match",
            }
            with mock.patch.object(MODULE, "_run") as run:
                run.return_value = subprocess.CompletedProcess(
                    [str(binary)],
                    0,
                    stdout=json.dumps(success, separators=(",", ":")) + "\n",
                    stderr="",
                )
                self.assertEqual(
                    MODULE._compare_current_artifact(
                        binary,
                        case_id="loader",
                        reference=reference,
                        actual=actual,
                    ),
                    success,
                )
                self.assertEqual(run.call_args.args[0], [str(binary)])
                controlled = {
                    key: value
                    for key, value in run.call_args.kwargs["environment"].items()
                    if key.startswith("CLONK_PRESENTATION_")
                }
                self.assertEqual(
                    controlled,
                    {
                        "CLONK_PRESENTATION_COMPARE": "1",
                        "CLONK_PRESENTATION_CASE": "loader",
                        "CLONK_PRESENTATION_REFERENCE": str(reference.resolve()),
                        "CLONK_PRESENTATION_ACTUAL": str(actual.resolve()),
                    },
                )

                run.return_value = subprocess.CompletedProcess(
                    [str(binary)],
                    0,
                    stdout=json.dumps({**success, "unchecked": True}),
                    stderr="",
                )
                with self.assertRaisesRegex(MODULE.AcquisitionFailure, "schema drift"):
                    MODULE._compare_current_artifact(
                        binary,
                        case_id="loader",
                        reference=reference,
                        actual=actual,
                    )

    def test_acquisition_compares_both_runs_and_retains_bound_attestations(self):
        with tempfile.TemporaryDirectory() as temporary:
            candidate = Path(temporary) / "candidate"
            binary = candidate / "builds/rust/binary"
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"rust-comparator")
            for run_id in MODULE.RUN_IDS:
                for engine in MODULE.ENGINE_IDS:
                    artifacts = candidate / run_id / engine / "artifacts"
                    artifacts.mkdir(parents=True)
                    for case_id in MODULE.CASE_IDS:
                        suffix = (
                            "layout.json"
                            if case_id in MODULE.LAYOUT_CASE_IDS
                            else "png"
                        )
                        (artifacts / f"{case_id}.{suffix}").write_bytes(b"artifact")

            def comparison_result(_command, **kwargs):
                case_id = kwargs["environment"]["CLONK_PRESENTATION_CASE"]
                result = {
                    "schema": "clonk-rs/presentation-comparison/v1",
                    "case_id": case_id,
                    "comparison": (
                        "layout" if case_id in MODULE.LAYOUT_CASE_IDS else "pixel"
                    ),
                    "status": "match",
                }
                return subprocess.CompletedProcess(
                    [str(binary)],
                    0,
                    stdout=json.dumps(result, separators=(",", ":")) + "\n",
                    stderr="",
                )

            with mock.patch.object(MODULE, "_run", side_effect=comparison_result) as run:
                comparisons = MODULE._run_acquisition_comparisons(candidate, binary)

            self.assertEqual(run.call_count, 2 * len(MODULE.CASE_IDS))
            self.assertEqual(
                [comparison["id"] for comparison in comparisons],
                list(MODULE.RUN_IDS),
            )
            for comparison_run in comparisons:
                run_id = comparison_run["id"]
                self.assertEqual(
                    [case["id"] for case in comparison_run["cases"]],
                    list(MODULE.CASE_IDS),
                )
                for case in comparison_run["cases"]:
                    case_id = case["id"]
                    receipt = candidate / case["receipt"]["path"]
                    attestation = json.loads(receipt.read_text(encoding="utf-8"))
                    suffix = (
                        "layout.json"
                        if case_id in MODULE.LAYOUT_CASE_IDS
                        else "png"
                    )
                    self.assertEqual(
                        attestation,
                        {
                            "schema": MODULE.COMPARISON_ATTESTATION_SCHEMA,
                            "run_id": run_id,
                            "case_id": case_id,
                            "comparison": case["comparison"],
                            "comparator": {
                                "schema": MODULE.COMPARISON_RECEIPT_SCHEMA,
                                "binary_sha256": sha256(binary),
                            },
                            "reference": {
                                "path": f"{run_id}/cpp/artifacts/{case_id}.{suffix}",
                                "sha256": case["reference"]["sha256"],
                            },
                            "actual": {
                                "path": f"{run_id}/rust/artifacts/{case_id}.{suffix}",
                                "sha256": case["actual"]["sha256"],
                            },
                            "stdout": json.dumps(
                                {
                                    "schema": MODULE.COMPARISON_RECEIPT_SCHEMA,
                                    "case_id": case_id,
                                    "comparison": case["comparison"],
                                    "status": "match",
                                },
                                separators=(",", ":"),
                            )
                            + "\n",
                        },
                    )
                    self.assertEqual(
                        case["reference"]["path"],
                        f"{run_id}/cpp/artifacts/{case_id}."
                        + ("layout.json" if case_id in MODULE.LAYOUT_CASE_IDS else "png"),
                    )

    def test_final_acquisition_validation_uses_the_retained_rust_binary(self):
        with tempfile.TemporaryDirectory() as temporary:
            candidate = Path(temporary) / "candidate"
            binary = candidate / "builds/rust/binary"
            binary.parent.mkdir(parents=True)
            binary.write_bytes(b"trusted comparator")
            binary.chmod(0o600)

            trusted = MODULE._acquisition_trusted_comparator(candidate)

            self.assertEqual(trusted, binary)
            self.assertTrue(trusted.stat().st_mode & stat.S_IXUSR)

    def test_validation_reruns_every_pair_with_the_trusted_comparator(self):
        with tempfile.TemporaryDirectory() as temporary:
            candidate = Path(temporary) / "candidate"
            fixture = write_v2_candidate(candidate)
            binary = candidate / fixture["index"]["builds"]["rust"]["binary"]["path"]

            def comparison_result(_command, **kwargs):
                case_id = kwargs["environment"]["CLONK_PRESENTATION_CASE"]
                actual = Path(
                    kwargs["environment"]["CLONK_PRESENTATION_ACTUAL"]
                )
                is_negative_canary = (
                    case_id == "network-lobby" and actual.name == "loader.png"
                ) or (
                    case_id == "startup-main"
                    and "clonk-presentation-layout-canary-" in str(actual)
                )
                if is_negative_canary:
                    term = "layout" if case_id in MODULE.LAYOUT_CASE_IDS else "pixel"
                    raise MODULE.AcquisitionFailure(
                        f"command failed (1): {term} mismatch: intentional canary"
                    )
                result = {
                    "schema": MODULE.COMPARISON_RECEIPT_SCHEMA,
                    "case_id": case_id,
                    "comparison": (
                        "layout" if case_id in MODULE.LAYOUT_CASE_IDS else "pixel"
                    ),
                    "status": "match",
                }
                return subprocess.CompletedProcess(
                    [str(binary)],
                    0,
                    stdout=json.dumps(result, separators=(",", ":")) + "\n",
                    stderr="",
                )

            with mock.patch.object(MODULE, "_run", side_effect=comparison_result) as run:
                MODULE.validate_candidate_output(
                    candidate,
                    expected_fields=fixture["expected_fields"],
                    expected_case_specs=fixture["case_specs"],
                    trusted_patch=fixture["trusted_patch"],
                    trusted_launcher=fixture["trusted_launcher"],
                    trusted_comparator=binary,
                )

            self.assertEqual(
                run.call_count,
                len(MODULE.RUN_IDS) * len(MODULE.CASE_IDS) + 2,
            )

    def test_negative_canaries_reject_an_always_match_comparator(self):
        root = Path("/accepted")
        comparator = Path("/trusted/clonk-app")
        success = (
            {
                "schema": MODULE.COMPARISON_RECEIPT_SCHEMA,
                "case_id": "network-lobby",
                "comparison": "pixel",
                "status": "match",
            },
            "match\n",
        )
        with (
            mock.patch.object(MODULE, "_regular_file"),
            mock.patch.object(
                MODULE,
                "_run_presentation_comparator",
                return_value=success,
            ),
        ):
            with self.assertRaisesRegex(
                MODULE.AcquisitionFailure,
                "accepted a pixel negative canary",
            ):
                MODULE._require_comparator_negative_canaries(comparator, root)

    def test_layout_negative_canary_changes_structure_without_changing_screen(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            artifacts = root / "run-1/cpp/artifacts"
            artifacts.mkdir(parents=True)
            for name in ("network-lobby.png", "loader.png"):
                (artifacts / name).write_bytes(b"png")
            for case_id in ("startup-main", "startup-about"):
                (artifacts / f"{case_id}.layout.json").write_text(
                    json.dumps(
                        {
                            "screen": case_id,
                            "elements": [
                                {
                                    "path": "startup/main/root",
                                    "rect": {"x": 0, "y": 0, "width": 10, "height": 10},
                                    "visible": True,
                                }
                            ],
                        }
                    ),
                    encoding="utf-8",
                )

            def comparison(_binary, *, case_id, reference, actual):
                if case_id == "network-lobby":
                    self.assertEqual(reference.name, "network-lobby.png")
                    self.assertEqual(actual.name, "loader.png")
                    raise MODULE.AcquisitionFailure(
                        "command failed (1): pixel mismatch: intentional canary"
                    )
                reference_value = json.loads(reference.read_text(encoding="utf-8"))
                actual_value = json.loads(actual.read_text(encoding="utf-8"))
                self.assertEqual(reference_value["screen"], "startup-main")
                self.assertEqual(actual_value["screen"], "startup-main")
                return ({"status": "match"}, "match\n")

            with mock.patch.object(
                MODULE,
                "_run_presentation_comparator",
                side_effect=comparison,
            ):
                with self.assertRaisesRegex(
                    MODULE.AcquisitionFailure,
                    "accepted a layout negative canary",
                ):
                    MODULE._require_comparator_negative_canaries(
                        Path("/trusted/clonk-app"),
                        root,
                    )

    def test_comparator_crash_does_not_satisfy_a_negative_canary(self):
        with (
            mock.patch.object(MODULE, "_regular_file"),
            mock.patch.object(
                MODULE,
                "_run_presentation_comparator",
                side_effect=MODULE.AcquisitionFailure("command timed out"),
            ),
        ):
            with self.assertRaisesRegex(
                MODULE.AcquisitionFailure,
                "did not report a pixel mismatch",
            ):
                MODULE._require_comparator_negative_canaries(
                    Path("/trusted/clonk-app"),
                    Path("/accepted"),
                )


def finalized_contract_values():
    manifest = json.loads(
        (REPOSITORY / MODULE.CAPTURE_MANIFEST_SOURCE_PATH).read_text(
            encoding="utf-8"
        )
    )
    for screen in manifest["screens"]:
        screen["status"] = "captured"
        screen.pop("blocker", None)
        suffix = "layout.json" if screen["comparison"] == "layout" else "png"
        screen["evidence"] = {
            engine: (
                "compat/presentation/oracle/v1/run-1/"
                f"{engine}/artifacts/{screen['id']}.{suffix}"
            )
            for engine in MODULE.ENGINE_IDS
        }
    profile = json.loads(
        (REPOSITORY / "compat/profile.json").read_text(encoding="utf-8")
    )
    lifecycle = next(
        entry
        for entry in profile["promise"]["presentation"]["evidence"]
        if entry["value"]
        in {"clonk-org/clonk-rs#587", MODULE.FINAL_PRESENTATION_GATE_EVIDENCE}
    )
    lifecycle.update(
        {
            "kind": "test",
            "value": MODULE.FINAL_PRESENTATION_GATE_EVIDENCE,
            "status": "held",
            "note": "Required current capture landing row.",
        }
    )
    return manifest, profile


class ProvenanceAndAcceptanceTests(unittest.TestCase):
    def test_final_lifecycle_rejects_evidence_detached_from_its_screen(self):
        manifest, profile = finalized_contract_values()
        MODULE._validate_final_presentation_lifecycle(manifest, profile)

        detached = copy.deepcopy(manifest)
        detached["screens"][0]["evidence"]["cpp"] = detached["screens"][1][
            "evidence"
        ]["cpp"]
        with self.assertRaisesRegex(
            MODULE.AcquisitionFailure,
            "startup-main.*evidence",
        ):
            MODULE._validate_final_presentation_lifecycle(detached, profile)

        pending = copy.deepcopy(manifest)
        pending["screens"][0]["status"] = "pending"
        with self.assertRaisesRegex(
            MODULE.AcquisitionFailure,
            "startup-main.*not captured",
        ):
            MODULE._validate_final_presentation_lifecycle(pending, profile)

        duplicate = copy.deepcopy(manifest)
        duplicate["screens"].append(copy.deepcopy(duplicate["screens"][0]))
        with self.assertRaisesRegex(
            MODULE.AcquisitionFailure,
            "screen identity or order drift",
        ):
            MODULE._validate_final_presentation_lifecycle(duplicate, profile)

        pending_profile = copy.deepcopy(profile)
        lifecycle = next(
            entry
            for entry in pending_profile["promise"]["presentation"]["evidence"]
            if entry["value"] == MODULE.FINAL_PRESENTATION_GATE_EVIDENCE
        )
        lifecycle.update(
            {
                "kind": "issue",
                "value": "clonk-org/clonk-rs#587",
                "status": "pending",
            }
        )
        with self.assertRaisesRegex(
            MODULE.AcquisitionFailure,
            "lifecycle is not held by the live gate",
        ):
            MODULE._validate_final_presentation_lifecycle(manifest, pending_profile)

    def test_accepted_verifier_reuses_a_prebuilt_current_comparator(self):
        repository = Path("/checkout")
        comparator = repository / "target/debug/clonk-app"
        context = (comparator, "a" * 40, "b" * 40, {"schema": "inventory"})
        with (
            mock.patch.object(MODULE, "_regular_file"),
            mock.patch.object(MODULE, "_build_trusted_current_comparator") as build,
        ):
            selected = MODULE._select_trusted_current_comparator(
                repository,
                context,
            )

        self.assertEqual(selected, context)
        build.assert_not_called()

    def test_verify_accepted_index_selects_nonexecuting_provenance_validation(self):
        repository = Path("/checkout")
        validated = [repository / "compat/presentation/oracle/v1/index.json"]
        with mock.patch.object(
            MODULE,
            "verify_accepted_output",
            return_value=validated,
        ) as verify:
            self.assertEqual(MODULE.verify_accepted_index(repository), validated)

        verify.assert_called_once_with(repository, rerun_comparisons=False)

    def test_capture_contract_digest_excludes_lifecycle_but_binds_terms(self):
        with tempfile.TemporaryDirectory() as temporary:
            manifest_path = Path(temporary) / "presentation_captures.json"
            manifest = json.loads(
                (REPOSITORY / "compat/presentation_captures.json").read_text(
                    encoding="utf-8"
                )
            )
            manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
            original = MODULE.capture_manifest_contract_sha256(manifest_path)

            manifest["screens"][0]["status"] = "captured"
            manifest["screens"][0]["evidence"] = {
                "cpp": "oracle/run-1/cpp/startup-main.layout.json",
                "rust": "oracle/run-1/rust/startup-main.layout.json",
            }
            manifest["screens"][0].pop("blocker", None)
            manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
            self.assertEqual(
                MODULE.capture_manifest_contract_sha256(manifest_path), original
            )

            manifest["tolerance"]["cpu_max_channel_delta"] = 1
            manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
            self.assertNotEqual(
                MODULE.capture_manifest_contract_sha256(manifest_path), original
            )

            manifest["tolerance"]["cpu_max_channel_delta"] = 0
            manifest["capture"]["pointer"]["position"] = [33, 32]
            manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
            with self.assertRaisesRegex(
                MODULE.AcquisitionFailure, "capture pointer input drift"
            ):
                MODULE.capture_manifest_contract_sha256(manifest_path)

    def test_profile_contract_normalizes_pending_issue_and_held_verifier_lifecycle(self):
        profile = json.loads(
            (REPOSITORY / "compat/profile.json").read_text(encoding="utf-8")
        )
        original = MODULE.compat_profile_contract_value_sha256(profile)
        evidence = profile["promise"]["presentation"]["evidence"]
        lifecycle = next(
            entry
            for entry in evidence
            if entry["value"] == MODULE.FINAL_PRESENTATION_GATE_EVIDENCE
        )
        pending_profile = copy.deepcopy(profile)
        pending = next(
            entry
            for entry in pending_profile["promise"]["presentation"]["evidence"]
            if entry["value"] == MODULE.FINAL_PRESENTATION_GATE_EVIDENCE
        )
        pending.update(
            {
                "kind": "issue",
                "value": "clonk-org/clonk-rs#587",
                "status": "pending",
                "note": "Capture evidence remains pending.",
            }
        )
        self.assertEqual(
            MODULE.compat_profile_contract_value_sha256(pending_profile), original
        )
        evidence.append(copy.deepcopy(pending))
        with self.assertRaisesRegex(MODULE.AcquisitionFailure, "exactly one.*lifecycle"):
            MODULE.compat_profile_contract_value_sha256(profile)
        evidence.pop()
        invalid = copy.deepcopy(pending_profile)
        invalid_entry = next(
            entry
            for entry in invalid["promise"]["presentation"]["evidence"]
            if entry["value"] == "clonk-org/clonk-rs#587"
        )
        invalid_entry["status"] = "held"
        with self.assertRaisesRegex(
            MODULE.AcquisitionFailure, "identity/status drift"
        ):
            MODULE.compat_profile_contract_value_sha256(invalid)
        self.assertEqual(lifecycle["status"], "held")
        profile["promise"]["presentation"]["statement"] += " changed"
        self.assertNotEqual(
            MODULE.compat_profile_contract_value_sha256(profile),
            original,
        )

    def test_normalization_records_the_corrected_cpp_readback_coordinate(self):
        # The pinned oracle reads row `realHgt - y` at src/C4Surface.cpp:434;
        # the capture-only patch must correct that coordinate, not crop/shift
        # the resulting artifact into appearing comparable.
        self.assertEqual(
            MODULE.EXPECTED_NORMALIZATION,
            {
                "cpp_savepng_readback": "real-height-minus-one-minus-y",
                "pointer_input": {
                    "position": [32, 32],
                    "button": "none",
                    "modifiers": [],
                    "help": False,
                },
                "color": "rgb-or-rgba8-srgb",
            },
        )

    def test_v2_rejects_an_arbitrary_well_formed_patch_digest(self):
        with tempfile.TemporaryDirectory() as temporary:
            candidate = Path(temporary) / "candidate"
            fixture = write_v2_candidate(candidate)
            fixture["index"]["patch"]["sha256"] = "f" * 64

            with self.assertRaisesRegex(MODULE.AcquisitionFailure, "patch.*SHA-256"):
                MODULE.validate_provenance_index(
                    fixture["index"],
                    candidate,
                    expected_fields=fixture["expected_fields"],
                    expected_case_specs=fixture["case_specs"],
                    trusted_patch=fixture["trusted_patch"],
                    trusted_launcher=fixture["trusted_launcher"],
                )

    def test_v2_validates_every_declared_evidence_file(self):
        with tempfile.TemporaryDirectory() as temporary:
            candidate = Path(temporary) / "candidate"
            fixture = write_v2_candidate(candidate)

            validated = MODULE.validate_provenance_index(
                fixture["index"],
                candidate,
                expected_fields=fixture["expected_fields"],
                expected_case_specs=fixture["case_specs"],
                trusted_patch=fixture["trusted_patch"],
                trusted_launcher=fixture["trusted_launcher"],
            )

            self.assertEqual(
                {path.relative_to(candidate).as_posix() for path in validated},
                {
                    path.relative_to(candidate).as_posix()
                    for path in candidate.rglob("*")
                    if path.is_file() and path.name != "index.json"
                },
            )

    def test_v2_rejects_descriptive_json_instead_of_consumed_native_config(self):
        with tempfile.TemporaryDirectory() as temporary:
            candidate = Path(temporary) / "candidate"
            fixture = write_v2_candidate(candidate)
            native = candidate / "inputs/cpp.config"
            descriptive = candidate / "inputs/cpp-config.json"
            native.rename(descriptive)
            fixture["index"]["inputs"]["configs"]["cpp"]["path"] = (
                "inputs/cpp-config.json"
            )

            with self.assertRaisesRegex(MODULE.AcquisitionFailure, "native config"):
                MODULE.validate_provenance_index(
                    fixture["index"],
                    candidate,
                    expected_fields=fixture["expected_fields"],
                    expected_case_specs=fixture["case_specs"],
                    trusted_patch=fixture["trusted_patch"],
                    trusted_launcher=fixture["trusted_launcher"],
                )

    def test_v2_rejects_a_non_oracle_locale_contract(self):
        with tempfile.TemporaryDirectory() as temporary:
            candidate = Path(temporary) / "candidate"
            fixture = write_v2_candidate(candidate)
            fixture["case_specs"][0]["locale"]["charset"] = "UTF-8"

            with self.assertRaisesRegex(MODULE.AcquisitionFailure, "canonical locale"):
                MODULE.validate_provenance_index(
                    fixture["index"],
                    candidate,
                    expected_fields=fixture["expected_fields"],
                    expected_case_specs=fixture["case_specs"],
                    trusted_patch=fixture["trusted_patch"],
                    trusted_launcher=fixture["trusted_launcher"],
                )

    def test_v2_rejects_copied_runs_with_the_same_launcher_nonce(self):
        with tempfile.TemporaryDirectory() as temporary:
            candidate = Path(temporary) / "candidate"
            fixture = write_v2_candidate(candidate)
            fixture["index"]["runs"][1]["nonce"] = fixture["index"]["runs"][0][
                "nonce"
            ]

            with self.assertRaisesRegex(MODULE.AcquisitionFailure, "distinct.*nonce"):
                MODULE.validate_provenance_index(
                    fixture["index"],
                    candidate,
                    expected_fields=fixture["expected_fields"],
                    expected_case_specs=fixture["case_specs"],
                    trusted_patch=fixture["trusted_patch"],
                    trusted_launcher=fixture["trusted_launcher"],
                )

    def test_v2_rejects_a_missing_per_case_engine_receipt(self):
        with tempfile.TemporaryDirectory() as temporary:
            candidate = Path(temporary) / "candidate"
            fixture = write_v2_candidate(candidate)
            (candidate / "run-2/rust/receipts/gameplay.json").unlink()

            with self.assertRaisesRegex(MODULE.AcquisitionFailure, "receipt.*regular file"):
                MODULE.validate_provenance_index(
                    fixture["index"],
                    candidate,
                    expected_fields=fixture["expected_fields"],
                    expected_case_specs=fixture["case_specs"],
                    trusted_patch=fixture["trusted_patch"],
                    trusted_launcher=fixture["trusted_launcher"],
                )

    def test_v2_rejects_an_unknown_engine_receipt_key_even_when_rehashed(self):
        with tempfile.TemporaryDirectory() as temporary:
            candidate = Path(temporary) / "candidate"
            fixture = write_v2_candidate(candidate)
            rewrite_engine_receipt(
                candidate,
                fixture["index"],
                "run-1",
                "cpp",
                "startup-main",
                lambda receipt: receipt.update(passed=True),
            )

            with self.assertRaisesRegex(MODULE.AcquisitionFailure, "receipt schema drift"):
                MODULE.validate_provenance_index(
                    fixture["index"],
                    candidate,
                    expected_fields=fixture["expected_fields"],
                    expected_case_specs=fixture["case_specs"],
                    trusted_patch=fixture["trusted_patch"],
                    trusted_launcher=fixture["trusted_launcher"],
                )

    def test_v2_requires_every_receipt_to_bind_the_network_fixture(self):
        with tempfile.TemporaryDirectory() as temporary:
            candidate = Path(temporary) / "candidate"
            fixture = write_v2_candidate(candidate)
            rewrite_engine_receipt(
                candidate,
                fixture["index"],
                "run-1",
                "rust",
                "loader",
                lambda receipt: receipt.pop("network_references_sha256"),
            )

            with self.assertRaisesRegex(MODULE.AcquisitionFailure, "receipt schema drift"):
                MODULE.validate_provenance_index(
                    fixture["index"],
                    candidate,
                    expected_fields=fixture["expected_fields"],
                    expected_case_specs=fixture["case_specs"],
                    trusted_patch=fixture["trusted_patch"],
                    trusted_launcher=fixture["trusted_launcher"],
                )

    def test_v2_rejects_an_unindexed_extra_file(self):
        with tempfile.TemporaryDirectory() as temporary:
            candidate = Path(temporary) / "candidate"
            fixture = write_v2_candidate(candidate)
            (candidate / "run-1/unindexed.txt").write_text("not evidence", encoding="utf-8")

            with self.assertRaisesRegex(MODULE.AcquisitionFailure, "file inventory drift"):
                MODULE.validate_provenance_index(
                    fixture["index"],
                    candidate,
                    expected_fields=fixture["expected_fields"],
                    expected_case_specs=fixture["case_specs"],
                    trusted_patch=fixture["trusted_patch"],
                    trusted_launcher=fixture["trusted_launcher"],
                )

    def test_v2_rejects_a_receipt_for_a_different_case(self):
        with tempfile.TemporaryDirectory() as temporary:
            candidate = Path(temporary) / "candidate"
            fixture = write_v2_candidate(candidate)
            rewrite_engine_receipt(
                candidate,
                fixture["index"],
                "run-1",
                "cpp",
                "startup-main",
                lambda receipt: receipt.update(case_id="startup-options"),
            )

            with self.assertRaisesRegex(MODULE.AcquisitionFailure, "case identity"):
                MODULE.validate_provenance_index(
                    fixture["index"],
                    candidate,
                    expected_fields=fixture["expected_fields"],
                    expected_case_specs=fixture["case_specs"],
                    trusted_patch=fixture["trusted_patch"],
                    trusted_launcher=fixture["trusted_launcher"],
                )

    def test_v2_rejects_a_receipt_at_the_wrong_checkpoint(self):
        with tempfile.TemporaryDirectory() as temporary:
            candidate = Path(temporary) / "candidate"
            fixture = write_v2_candidate(candidate)
            rewrite_engine_receipt(
                candidate,
                fixture["index"],
                "run-1",
                "rust",
                "gameplay",
                lambda receipt: receipt["frame"].update(checkpoint="stale/frame"),
            )

            with self.assertRaisesRegex(MODULE.AcquisitionFailure, "frame/checkpoint"):
                MODULE.validate_provenance_index(
                    fixture["index"],
                    candidate,
                    expected_fields=fixture["expected_fields"],
                    expected_case_specs=fixture["case_specs"],
                    trusted_patch=fixture["trusted_patch"],
                    trusted_launcher=fixture["trusted_launcher"],
                )

    def test_v2_rejects_a_comparator_receipt_that_did_not_match(self):
        with tempfile.TemporaryDirectory() as temporary:
            candidate = Path(temporary) / "candidate"
            fixture = write_v2_candidate(candidate)
            comparison = next(
                case
                for case in fixture["index"]["comparisons"][0]["cases"]
                if case["id"] == "gameplay"
            )
            receipt = candidate / comparison["receipt"]["path"]
            attestation = json.loads(receipt.read_text(encoding="utf-8"))
            attestation["stdout"] = json.dumps(
                {
                    "schema": MODULE.COMPARISON_RECEIPT_SCHEMA,
                    "case_id": "gameplay",
                    "comparison": "layout",
                    "status": "mismatch",
                },
                separators=(",", ":"),
            ) + "\n"
            receipt.write_text(
                json.dumps(attestation, sort_keys=True),
                encoding="utf-8",
            )
            comparison["receipt"] = artifact_metadata(
                candidate,
                comparison["receipt"]["path"],
            )

            with self.assertRaisesRegex(
                MODULE.AcquisitionFailure, "comparison stdout is not exact"
            ):
                MODULE.validate_provenance_index(
                    fixture["index"],
                    candidate,
                    expected_fields=fixture["expected_fields"],
                    expected_case_specs=fixture["case_specs"],
                    trusted_patch=fixture["trusted_patch"],
                    trusted_launcher=fixture["trusted_launcher"],
                )

    def test_v2_rejects_a_receipt_bound_to_different_content(self):
        with tempfile.TemporaryDirectory() as temporary:
            candidate = Path(temporary) / "candidate"
            fixture = write_v2_candidate(candidate)
            rewrite_engine_receipt(
                candidate,
                fixture["index"],
                "run-2",
                "cpp",
                "loader",
                lambda receipt: receipt.update(content_tree="0" * 40),
            )

            with self.assertRaisesRegex(MODULE.AcquisitionFailure, "content tree"):
                MODULE.validate_provenance_index(
                    fixture["index"],
                    candidate,
                    expected_fields=fixture["expected_fields"],
                    expected_case_specs=fixture["case_specs"],
                    trusted_patch=fixture["trusted_patch"],
                    trusted_launcher=fixture["trusted_launcher"],
                )

    def test_v2_rejects_a_rust_receipt_without_the_compatibility_profile(self):
        with tempfile.TemporaryDirectory() as temporary:
            candidate = Path(temporary) / "candidate"
            fixture = write_v2_candidate(candidate)
            rewrite_engine_receipt(
                candidate,
                fixture["index"],
                "run-1",
                "rust",
                "hud",
                lambda receipt: receipt.update(profile="normal"),
            )

            with self.assertRaisesRegex(MODULE.AcquisitionFailure, "profile"):
                MODULE.validate_provenance_index(
                    fixture["index"],
                    candidate,
                    expected_fields=fixture["expected_fields"],
                    expected_case_specs=fixture["case_specs"],
                    trusted_patch=fixture["trusted_patch"],
                    trusted_launcher=fixture["trusted_launcher"],
                )

    def test_v2_rejects_a_receipt_for_a_different_binary(self):
        with tempfile.TemporaryDirectory() as temporary:
            candidate = Path(temporary) / "candidate"
            fixture = write_v2_candidate(candidate)
            rewrite_engine_receipt(
                candidate,
                fixture["index"],
                "run-2",
                "rust",
                "evaluation",
                lambda receipt: receipt.update(binary_sha256="0" * 64),
            )

            with self.assertRaisesRegex(MODULE.AcquisitionFailure, "binary SHA-256"):
                MODULE.validate_provenance_index(
                    fixture["index"],
                    candidate,
                    expected_fields=fixture["expected_fields"],
                    expected_case_specs=fixture["case_specs"],
                    trusted_patch=fixture["trusted_patch"],
                    trusted_launcher=fixture["trusted_launcher"],
                )

    def test_v2_rejects_a_missing_launcher_receipt(self):
        with tempfile.TemporaryDirectory() as temporary:
            candidate = Path(temporary) / "candidate"
            fixture = write_v2_candidate(candidate)
            (candidate / "run-1/launcher-receipt.json").unlink()

            with self.assertRaisesRegex(
                MODULE.AcquisitionFailure, "launcher receipt.*regular file"
            ):
                MODULE.validate_provenance_index(
                    fixture["index"],
                    candidate,
                    expected_fields=fixture["expected_fields"],
                    expected_case_specs=fixture["case_specs"],
                    trusted_patch=fixture["trusted_patch"],
                    trusted_launcher=fixture["trusted_launcher"],
                )

    def test_v2_rejects_an_artifact_that_differs_from_its_engine_receipt(self):
        with tempfile.TemporaryDirectory() as temporary:
            candidate = Path(temporary) / "candidate"
            fixture = write_v2_candidate(candidate)
            (candidate / "run-2/cpp/artifacts/gameplay.png").write_bytes(
                png_bytes(sample_byte=1)
            )

            with self.assertRaisesRegex(MODULE.AcquisitionFailure, "artifact SHA-256"):
                MODULE.validate_provenance_index(
                    fixture["index"],
                    candidate,
                    expected_fields=fixture["expected_fields"],
                    expected_case_specs=fixture["case_specs"],
                    trusted_patch=fixture["trusted_patch"],
                    trusted_launcher=fixture["trusted_launcher"],
                )

    def test_v2_requires_each_engine_repeat_to_be_byte_identical(self):
        with tempfile.TemporaryDirectory() as temporary:
            candidate = Path(temporary) / "candidate"
            fixture = write_v2_candidate(candidate)
            artifact = candidate / "run-2/rust/artifacts/gameplay.png"
            artifact.write_bytes(png_bytes(sample_byte=1))
            rewrite_engine_receipt(
                candidate,
                fixture["index"],
                "run-2",
                "rust",
                "gameplay",
                lambda receipt: receipt["artifacts"].update(
                    png=artifact_metadata(
                        candidate, "run-2/rust/artifacts/gameplay.png"
                    )
                ),
            )

            with self.assertRaisesRegex(MODULE.AcquisitionFailure, "repeat.*differ"):
                MODULE.validate_provenance_index(
                    fixture["index"],
                    candidate,
                    expected_fields=fixture["expected_fields"],
                    expected_case_specs=fixture["case_specs"],
                    trusted_patch=fixture["trusted_patch"],
                    trusted_launcher=fixture["trusted_launcher"],
                )

    def test_v2_receipt_must_echo_every_case_input_binding(self):
        mutations = {
            "source tree": lambda receipt: receipt.update(source_tree="0" * 40),
            "config": lambda receipt: receipt.update(config_sha256="0" * 64),
            "player": lambda receipt: receipt.update(player_sha256="0" * 64),
            "locale": lambda receipt: receipt["locale"].update(language="DE"),
            "seeds": lambda receipt: receipt["seeds"]["presentation"].update(calls=1),
            "trigger": lambda receipt: receipt["trigger"].update(id="wrong-trigger"),
            "scenario": lambda receipt: receipt["scenario"].update(path="wrong.c4s"),
        }
        for label, mutate in mutations.items():
            with self.subTest(label=label), tempfile.TemporaryDirectory() as temporary:
                candidate = Path(temporary) / "candidate"
                fixture = write_v2_candidate(candidate)
                rewrite_engine_receipt(
                    candidate,
                    fixture["index"],
                    "run-1",
                    "cpp",
                    "startup-options",
                    mutate,
                )

                with self.assertRaisesRegex(MODULE.AcquisitionFailure, "binding mismatch"):
                    MODULE.validate_provenance_index(
                        fixture["index"],
                        candidate,
                        expected_fields=fixture["expected_fields"],
                        expected_case_specs=fixture["case_specs"],
                        trusted_patch=fixture["trusted_patch"],
                        trusted_launcher=fixture["trusted_launcher"],
                    )

    def test_v1_flat_index_is_not_accepted_by_v2(self):
        with tempfile.TemporaryDirectory() as temporary:
            artifact_root = Path(temporary) / "oracle"
            write_capture_set(artifact_root)
            index = provenance_index(artifact_root)
            index["schema"] = "clonk-rs/presentation-oracle/v1"

            with self.assertRaisesRegex(MODULE.AcquisitionFailure, "provenance index schema drift"):
                MODULE.validate_provenance_index(
                    index,
                    artifact_root,
                    expected_fields=v2_expected_provenance_fields(),
                )

    def test_candidate_validation_requires_two_runs_and_duplicate_safe_json(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "candidate"
            fixture = write_v2_candidate(output)

            validated = MODULE.validate_candidate_output(
                output,
                expected_fields=fixture["expected_fields"],
                expected_case_specs=fixture["case_specs"],
                trusted_patch=fixture["trusted_patch"],
                trusted_launcher=fixture["trusted_launcher"],
                trusted_comparator=fixture["trusted_comparator"],
            )
            self.assertEqual(
                {path.relative_to(output).as_posix() for path in validated},
                {
                    path.relative_to(output).as_posix()
                    for path in output.rglob("*")
                    if path.is_file()
                },
            )

            (output / "index.json").write_text(
                '{"schema":"one","schema":"two"}',
                encoding="utf-8",
            )
            with self.assertRaisesRegex(MODULE.AcquisitionFailure, "duplicate JSON key"):
                MODULE.validate_candidate_output(
                    output,
                    expected_fields=fixture["expected_fields"],
                    expected_case_specs=fixture["case_specs"],
                    trusted_patch=fixture["trusted_patch"],
                    trusted_launcher=fixture["trusted_launcher"],
                    trusted_comparator=fixture["trusted_comparator"],
                )

    def test_accept_copies_only_audited_artifacts_inputs_and_provenance(self):
        with tempfile.TemporaryDirectory() as temporary:
            temporary = Path(temporary)
            output = temporary / "candidate"
            fixture = write_v2_candidate(output)
            destination = temporary / "accepted"

            copied = MODULE.accept_candidate_output(
                output,
                destination,
                expected_fields=fixture["expected_fields"],
                expected_case_specs=fixture["case_specs"],
                trusted_patch=fixture["trusted_patch"],
                trusted_launcher=fixture["trusted_launcher"],
                trusted_comparator=fixture["trusted_comparator"],
            )

            self.assertTrue((destination / "index.json").is_file())
            self.assertTrue((destination / "run-1/cpp/artifacts/gameplay.png").is_file())
            self.assertTrue((destination / "run-2/rust/receipts/evaluation.json").is_file())
            self.assertTrue((destination / MODULE.PLAYER_RETAINED_PATH).is_file())
            self.assertTrue(
                (destination / MODULE.NETWORK_REFERENCES_RETAINED_PATH).is_file()
            )
            self.assertTrue(
                (destination / MODULE.RUST_SOURCE_INVENTORY_RETAINED_PATH).is_file()
            )
            self.assertFalse((destination / "builds/cpp/binary").exists())
            self.assertFalse((destination / "inputs/cpp-capture.patch").exists())
            self.assertFalse((destination / "launcher/acquire_presentation_oracle.py").exists())
            self.assertEqual(
                {
                    path.relative_to(destination).as_posix()
                    for path in destination.rglob("*")
                    if path.is_file()
                },
                {path.relative_to(destination).as_posix() for path in copied},
            )

            validated = MODULE.validate_candidate_output(
                destination,
                expected_fields=fixture["expected_fields"],
                expected_case_specs=fixture["case_specs"],
                trusted_patch=None,
                trusted_launcher=None,
                trusted_patch_sha256=sha256(fixture["trusted_patch"]),
                trusted_launcher_sha256=sha256(fixture["trusted_launcher"]),
                expected_source_inventory=fixture["source_inventory"],
                accepted_inventory=True,
                trusted_comparator=fixture["trusted_comparator"],
            )
            self.assertEqual(
                {path.relative_to(destination).as_posix() for path in validated},
                {
                    path.relative_to(destination).as_posix()
                    for path in destination.rglob("*")
                    if path.is_file()
                },
            )

    def test_accept_rejects_unequal_artifacts_after_all_self_claims_are_rehashed(self):
        with tempfile.TemporaryDirectory() as temporary:
            temporary = Path(temporary)
            candidate = temporary / "candidate"
            fixture = write_v2_candidate(candidate)
            index = fixture["index"]

            for run_id in MODULE.RUN_IDS:
                relative = f"{run_id}/rust/artifacts/network-lobby.png"
                (candidate / relative).write_bytes(png_bytes(sample_byte=1))
                new_artifact = artifact_metadata(candidate, relative)
                rewrite_engine_receipt(
                    candidate,
                    index,
                    run_id,
                    "rust",
                    "network-lobby",
                    lambda receipt, record=new_artifact: receipt["artifacts"].update(
                        png=record
                    ),
                )
                comparison_run = next(
                    entry for entry in index["comparisons"] if entry["id"] == run_id
                )
                comparison = next(
                    entry
                    for entry in comparison_run["cases"]
                    if entry["id"] == "network-lobby"
                )
                comparison["actual"] = copy.deepcopy(new_artifact)
                attestation_path = candidate / comparison["receipt"]["path"]
                attestation = json.loads(
                    attestation_path.read_text(encoding="utf-8")
                )
                attestation["actual"] = {
                    "path": new_artifact["path"],
                    "sha256": new_artifact["sha256"],
                }
                attestation_path.write_text(
                    json.dumps(attestation, sort_keys=True), encoding="utf-8"
                )
                comparison["receipt"] = artifact_metadata(
                    candidate, comparison["receipt"]["path"]
                )
            (candidate / "index.json").write_text(
                json.dumps(index, sort_keys=True), encoding="utf-8"
            )

            MODULE.validate_provenance_index(
                index,
                candidate,
                expected_fields=fixture["expected_fields"],
                expected_case_specs=fixture["case_specs"],
                trusted_patch=fixture["trusted_patch"],
                trusted_launcher=fixture["trusted_launcher"],
            )
            destination = temporary / "accepted"
            with self.assertRaisesRegex(MODULE.AcquisitionFailure, "command failed"):
                MODULE.accept_candidate_output(
                    candidate,
                    destination,
                    expected_fields=fixture["expected_fields"],
                    expected_case_specs=fixture["case_specs"],
                    trusted_patch=fixture["trusted_patch"],
                    trusted_launcher=fixture["trusted_launcher"],
                    trusted_comparator=fixture["trusted_comparator"],
                )
            self.assertFalse(destination.exists())

    def test_accept_does_not_create_destination_for_invalid_candidate(self):
        with tempfile.TemporaryDirectory() as temporary:
            temporary = Path(temporary)
            output = temporary / "candidate"
            fixture = write_v2_candidate(output)
            (output / "run-2/cpp/receipts/loader.json").unlink()
            destination = temporary / "accepted"

            with self.assertRaises(MODULE.AcquisitionFailure):
                MODULE.accept_candidate_output(
                    output,
                    destination,
                    expected_fields=fixture["expected_fields"],
                    expected_case_specs=fixture["case_specs"],
                    trusted_patch=fixture["trusted_patch"],
                    trusted_launcher=fixture["trusted_launcher"],
                    trusted_comparator=fixture["trusted_comparator"],
                )
            self.assertFalse(destination.exists())

    def test_accept_rejects_a_destination_nested_inside_the_candidate(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "candidate"
            fixture = write_v2_candidate(output)
            destination = output / "accepted"
            with self.assertRaisesRegex(
                MODULE.AcquisitionFailure, "inside candidate output"
            ):
                MODULE.accept_candidate_output(
                    output,
                    destination,
                    expected_fields=fixture["expected_fields"],
                    expected_case_specs=fixture["case_specs"],
                    trusted_patch=fixture["trusted_patch"],
                    trusted_launcher=fixture["trusted_launcher"],
                    expected_source_inventory=fixture["source_inventory"],
                    trusted_comparator=fixture["trusted_comparator"],
                )
            self.assertFalse(destination.exists())

    def test_accept_revalidates_copied_bytes_and_removes_a_corrupt_destination(self):
        with tempfile.TemporaryDirectory() as temporary:
            temporary = Path(temporary)
            output = temporary / "candidate"
            fixture = write_v2_candidate(output)
            destination = temporary / "accepted"
            copyfile = shutil.copyfile

            def corrupt_one_copy(source, target):
                result = copyfile(source, target)
                target = Path(target)
                if target == destination / "run-1/cpp/artifacts/gameplay.png":
                    target.write_bytes(b"corrupt after validation")
                return result

            with mock.patch.object(
                MODULE.shutil,
                "copyfile",
                side_effect=corrupt_one_copy,
            ):
                with self.assertRaises(MODULE.AcquisitionFailure):
                    MODULE.accept_candidate_output(
                        output,
                        destination,
                        expected_fields=fixture["expected_fields"],
                        expected_case_specs=fixture["case_specs"],
                        trusted_patch=fixture["trusted_patch"],
                        trusted_launcher=fixture["trusted_launcher"],
                        expected_source_inventory=fixture["source_inventory"],
                        trusted_comparator=fixture["trusted_comparator"],
                    )
            self.assertFalse(destination.exists())

    def test_verify_accepted_uses_squash_stable_current_inputs_not_recorded_commit(self):
        with tempfile.TemporaryDirectory() as temporary:
            repository = Path(temporary) / "repository"
            candidate = Path(temporary) / "candidate"
            repository.mkdir()
            fixture = write_v2_candidate(candidate)
            accepted = repository / "compat/presentation/oracle/v1"
            MODULE.accept_candidate_output(
                candidate,
                accepted,
                expected_fields=fixture["expected_fields"],
                expected_case_specs=fixture["case_specs"],
                trusted_patch=fixture["trusted_patch"],
                trusted_launcher=fixture["trusted_launcher"],
                trusted_comparator=fixture["trusted_comparator"],
            )
            current_comparator = Path(temporary) / "current-comparator"
            write_test_comparator(current_comparator)
            current_comparator.write_text(
                current_comparator.read_text(encoding="utf-8")
                + "# current squash-stable comparator build\n",
                encoding="utf-8",
            )
            current_comparator.chmod(0o755)
            self.assertNotEqual(
                sha256(current_comparator), sha256(fixture["trusted_comparator"])
            )
            manifest, profile = finalized_contract_values()
            source_hashes = {
                MODULE.CPP_CONFIG_SOURCE_PATH: fixture["index"]["inputs"]["configs"][
                    "cpp"
                ]["sha256"],
                MODULE.RUST_CONFIG_SOURCE_PATH: fixture["index"]["inputs"]["configs"][
                    "rust"
                ]["sha256"],
                MODULE.PLAYER_SOURCE_PATH: fixture["index"]["inputs"]["player"][
                    "sha256"
                ],
                MODULE.NETWORK_REFERENCES_SOURCE_PATH: fixture["index"]["inputs"][
                    "network_references"
                ]["sha256"],
                MODULE.CAPTURE_PATCH_SOURCE_PATH: sha256(fixture["trusted_patch"]),
                MODULE.LAUNCHER_SOURCE_PATH: sha256(fixture["trusted_launcher"]),
            }
            changed_inventory = copy.deepcopy(fixture["source_inventory"])
            changed_inventory["entries"][0]["sha256"] = "e" * 64
            changed_inventory["sha256"] = canonical_sha256(
                changed_inventory["entries"]
            )

            def committed_json(_repository, revision, relative):
                self.assertEqual(revision, "HEAD")
                return {
                    MODULE.CAPTURE_MANIFEST_SOURCE_PATH: manifest,
                    "compat/profile.json": profile,
                    MODULE.CASE_SPECS_SOURCE_PATH: fixture["case_specs"],
                }[relative]

            with (
                mock.patch.object(
                    MODULE,
                    "_build_trusted_current_comparator",
                    return_value=(
                        current_comparator,
                        "a" * 40,
                        "b" * 40,
                        fixture["source_inventory"],
                    ),
                ),
                mock.patch.object(MODULE, "verify_clean_source_checkpoint"),
                mock.patch.object(
                    MODULE,
                    "expected_provenance_fields",
                    return_value=copy.deepcopy(fixture["expected_fields"]),
                ) as expected,
                mock.patch.object(
                    MODULE,
                    "load_json_at_revision",
                    side_effect=committed_json,
                ),
                mock.patch.object(
                    MODULE,
                    "_git_blob_sha256",
                    side_effect=lambda _repository, revision, path: (
                        source_hashes[path] if revision == "HEAD" else self.fail(revision)
                    ),
                ),
                mock.patch.object(
                    MODULE,
                    "rust_source_input_inventory",
                    return_value=changed_inventory,
                ),
            ):
                validated = MODULE.verify_accepted_output(repository)

            self.assertTrue(validated)
            self.assertEqual(expected.call_args.kwargs["rust_revision"], "HEAD")
            self.assertFalse(expected.call_args.kwargs["require_oracle_head"])

    def test_cli_requires_an_explicit_output_directory(self):
        parser = MODULE.build_parser()
        with self.assertRaises(SystemExit):
            parser.parse_args(["validate"])
        arguments = parser.parse_args(
            [
                "validate",
                "--oracle-root",
                "/tmp/oracle",
                "--output-dir",
                "/tmp/candidate",
            ]
        )
        self.assertEqual(arguments.output_dir, Path("/tmp/candidate"))
        self.assertFalse(arguments.accept)

        acquire = parser.parse_args(
            [
                "acquire",
                "--oracle-root",
                "/tmp/oracle",
                "--output-dir",
                "/tmp/candidate",
                "--accept",
            ]
        )
        self.assertTrue(acquire.accept)
        accepted = parser.parse_args(
            ["verify-accepted", "--repo-root", "/tmp/repository"]
        )
        self.assertEqual(accepted.repo_root, Path("/tmp/repository"))
        accepted_index = parser.parse_args(
            ["verify-accepted-index", "--repo-root", "/tmp/repository"]
        )
        self.assertEqual(accepted_index.repo_root, Path("/tmp/repository"))
        current = parser.parse_args(
            [
                "verify-current",
                "--repo-root",
                "/tmp/repository",
                "--output-dir",
                "/tmp/current",
                "--profile",
                "test",
            ]
        )
        self.assertEqual(current.output_dir, Path("/tmp/current"))
        self.assertEqual(current.profile, "test")
        self.assertEqual(
            MODULE.rust_current_build_recipe(Path("/tmp/repository"), "test")[
                "commands"
            ][0]["argv"],
            [
                "cargo",
                "build",
                "--locked",
                "--profile",
                "test",
                "-p",
                "clonk-app",
                "--features",
                "presentation-capture",
            ],
        )


if __name__ == "__main__":
    unittest.main()
