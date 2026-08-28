#!/usr/bin/env python3

from __future__ import annotations

import os
import signal
import subprocess
import sys


TIMEOUT_STATUS = 124
TERMINATION_GRACE_SECONDS = 1


def terminate_process_tree(process: subprocess.Popen[bytes]) -> None:
    try:
        if os.name == "posix":
            os.killpg(process.pid, signal.SIGTERM)
        else:
            process.terminate()
    except ProcessLookupError:
        return

    try:
        process.wait(timeout=TERMINATION_GRACE_SECONDS)
        return
    except subprocess.TimeoutExpired:
        pass

    try:
        if os.name == "posix":
            os.killpg(process.pid, signal.SIGKILL)
        else:
            process.kill()
    except ProcessLookupError:
        return
    process.wait()


def main(arguments: list[str]) -> int:
    if len(arguments) < 2:
        print("usage: run_with_timeout.py SECONDS COMMAND [ARG ...]", file=sys.stderr)
        return 2

    timeout_seconds = float(arguments[0])
    if timeout_seconds <= 0:
        print("SECONDS must be positive", file=sys.stderr)
        return 2

    try:
        process = subprocess.Popen(
            arguments[1:],
            start_new_session=os.name == "posix",
        )
    except FileNotFoundError as error:
        print(error, file=sys.stderr)
        return 127

    try:
        return process.wait(timeout=timeout_seconds)
    except subprocess.TimeoutExpired:
        terminate_process_tree(process)
        return TIMEOUT_STATUS


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
