#!/usr/bin/env python3
"""Qualify the pinned group bridge against this tree's recorded Rust artifact."""

import argparse
import json
import os
from pathlib import Path
import re
import shlex
import shutil
import subprocess
import sys
import tempfile

from oracle_group_record import digest, verify_build


def compile_probe(port, oracle, build, out):
    # Keep the actual oracle parser and engine globals. Replace only main and
    # the bridge's open boundary in a separate executable for fault injection.
    link = shlex.split((build / "CMakeFiles/clonk.dir/link.txt").read_text())
    flags = dict(line.split(" = ", 1) for line in
                 (build / "CMakeFiles/clonk.dir/flags.make").read_text().splitlines()
                 if line.startswith(("CXX_DEFINES = ", "CXX_INCLUDES = ", "CXX_FLAGS = ")))
    compile_flags = [arg for key in ("CXX_DEFINES", "CXX_INCLUDES", "CXX_FLAGS")
                     for arg in shlex.split(flags[key])]
    pch = build / "CMakeFiles/clonk.dir/cmake_pch.hxx"
    replacements = {
        "CMakeFiles/clonk.dir/src/C4WinMain.cpp.o": (
            oracle / "src/C4WinMain.cpp", ["-Dmain=oracle_main"], out / "oracle-main.o"),
        "CMakeFiles/clonk.dir/src/rust/RustGroupBridge.cpp.o": (
            oracle / "src/rust/RustGroupBridge.cpp",
            ["-Dlc_group_open=probe_lc_group_open"], out / "bridge.o"),
    }
    for original, (source, extra, target) in replacements.items():
        if link.count(original) != 1:
            raise ValueError(f"oracle link does not contain exactly one {original}")
        subprocess.run(
            [link[0], *compile_flags, "-include", str(pch), *extra,
             "-c", str(source), "-o", str(target)], cwd=build, check=True,
        )
        link[link.index(original)] = str(target)
    probe = out / "probe.o"
    subprocess.run(
        [link[0], *compile_flags, "-include", str(pch), "-c",
         str(port / "parity/bridge/group_differential_probe.cpp"), "-o", str(probe)],
        cwd=build, check=True,
    )
    link.insert(1, str(probe))
    executable = out / "group-differential-probe"
    link[link.index("-o") + 1] = str(executable)
    subprocess.run(link, cwd=build, check=True)
    (out / "probe-link.json").write_text(json.dumps(link, indent=2) + "\n")
    return executable


def run_fixtures(executable, out, check_leaks):
    fixture_root = out / "fixtures"
    fixture_root.mkdir()
    results = []

    def run(name, *arguments):
        result = subprocess.run([str(executable), *map(str, arguments)],
                                cwd=out, capture_output=True, text=True)
        (out / f"{name}.log").write_text(result.stdout + result.stderr)
        if result.returncode:
            raise ValueError(f"{name} exited {result.returncode}: {result.stdout}{result.stderr}")
        return result.stdout

    normal = fixture_root / "normal"
    (normal / "Child").mkdir(parents=True)
    (normal / "Alpha.txt").write_bytes(b"alpha\x00\xff")
    (normal / "Beta.txt").write_bytes(b"beta")
    (normal / "Empty.txt").write_bytes(b"")
    (normal / "Child/Inner name.txt").write_bytes(b"nested data\x00")
    empty = fixture_root / "empty"
    empty.mkdir()
    fixtures = {"normal": normal, "empty": empty}
    for name in ("missing", "additional", "size", "type"):
        folder = fixture_root / name
        shutil.copytree(normal, folder)
        fixtures[name] = folder
    (fixtures["missing"] / "Beta.txt").unlink()
    (fixtures["additional"] / "Extra.txt").write_bytes(b"extra")
    (fixtures["size"] / "Alpha.txt").write_bytes(b"longer file")
    shutil.rmtree(fixtures["type"] / "Child")
    (fixtures["type"] / "Child").write_bytes(b"file instead of a child group")
    for name, folder in list(fixtures.items()):
        packed = fixture_root / f"{name}.c4g"
        run(f"pack-{name}", "pack", folder, packed)
        fixtures[f"{name}-packed"] = packed

    def compare(name, legacy, rust, expected=""):
        output = run(name, "compare", legacy, rust)
        reports = [line for line in output.splitlines() if "Rust group validation" in line]
        if expected:
            if len(reports) != 1 or expected not in reports[0]:
                raise ValueError(f"{name}: expected one '{expected}' report, got {reports}")
        elif reports:
            raise ValueError(f"{name}: agreeing inputs reported {reports}")
        results.append({"fixture": name, "report": reports})
        if check_leaks:
            environment = dict(os.environ, MallocStackLogging="1")
            leak = subprocess.run(
                ["leaks", "--atExit", "--", str(executable), "compare", str(legacy), str(rust)],
                cwd=out, env=environment, capture_output=True, text=True,
            )
            text = leak.stdout + leak.stderr
            (out / f"{name}-leaks.log").write_text(text)
            summary = re.search(r"Process \d+: (\d+) leaks? for .*", text)
            if not summary:
                raise ValueError(f"{name}: leaks produced no summary")
            if re.search(r"lc_group_|RustGroupBridge|clonk_resources|CompareABI", text):
                raise ValueError(f"{name}: a leak reaches the group bridge")
            results[-1]["leaks"] = summary.group(0)
        print(f"PASS {name}", flush=True)

    for name in ("normal", "empty", "normal-packed", "empty-packed"):
        compare(name, fixtures[name], fixtures[name])
    for name in ("normal", "normal-packed"):
        child = fixtures[name] / "Child"
        compare(f"{name}-nested", child, child)
    for name, expected in (
        ("missing", "entries missing from Rust view: Beta.txt"),
        ("additional", "additional entries reported by Rust: Extra.txt"),
        ("size", "size mismatch for entries: Alpha.txt"),
        ("type", "entry type mismatch for: Child"),
    ):
        compare(name, fixtures["normal-packed"], fixtures[f"{name}-packed"], expected)
    return results


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--oracle-root", type=Path, required=True)
    parser.add_argument("--build-dir", default="build-group")
    parser.add_argument("--out", type=Path)
    parser.add_argument("--leaks", action="store_true")
    args = parser.parse_args()
    port = Path(__file__).resolve().parents[2]
    oracle = args.oracle_root.resolve()
    build = oracle / args.build_dir
    record = verify_build(port, oracle, build)
    out = args.out or Path(tempfile.mkdtemp(prefix="group-differential-"))
    out.mkdir(parents=True, exist_ok=True)
    out = out.resolve()
    if args.leaks and not shutil.which("leaks"):
        raise ValueError("--leaks requires the macOS leaks tool")
    executable = compile_probe(port, oracle, build, out)
    results = run_fixtures(executable, out, args.leaks)
    verify_build(port, oracle, build)
    (out / "result.json").write_text(json.dumps({
        "build": record, "probe_sha256": digest(executable), "fixtures": results,
    }, indent=2) + "\n")
    print(f"group differential: {len(results)} fixtures passed; evidence: {out}")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        sys.exit(f"group differential: {error}")
