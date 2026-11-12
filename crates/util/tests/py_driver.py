"""Differential-test driver for the mn-util Rust port.

Reads a JSON array of cases ({"fn": <mininet.util function name>, "args":
[...]}) from stdin, runs each case against the ORIGINAL mininet.util
functions, and writes a JSON array of results to stdout. Python exceptions
are reported as {"__error__": "<ExceptionClass>"}. The `retry` function is
special-cased (its give-up path calls exit(1), i.e. raises SystemExit) and
reports ["ok"|"exit", <number of fn calls>].
"""

import json
import os
import sys

# Make the repo's mininet package importable regardless of cwd.
sys.path.insert(
    0, os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
)

from mininet import util  # noqa: E402


def run_case(case):
    fn = case["fn"]
    args = case["args"]
    try:
        if fn == "retry":
            retries, delay, fail_times = args
            calls = [0]

            def f():
                calls[0] += 1
                return calls[0] > fail_times

            try:
                util.retry(retries, delay, f)
                return ["ok", calls[0]]
            except SystemExit:
                return ["exit", calls[0]]
        f = getattr(util, fn)
        result = f(*args)
        if fn == "irange":
            result = list(result)
        elif fn == "splitArgs":
            name, pos, kw = result
            result = [name, pos, kw]
        return result
    except Exception as exc:  # noqa: BLE001 - report any Python exception
        return {"__error__": type(exc).__name__}


def main():
    cases = json.load(sys.stdin)
    results = [run_case(c) for c in cases]
    json.dump(results, sys.stdout)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
