#!/usr/bin/env python3
"""Measure how real .env loaders read each fixture in src/assets/dialects.json.

RM-04: every value in that table must come from running the loader, never from
documentation or memory. This runs each one and exits 1 when a measured value
differs from the table, printing what it measured so a person can update it.

Needs: python-dotenv importable by $PYTHON, `dotenv` and `dotenv-expand` under
$NODE_MODULES, `docker compose`, and an evnx binary as $EVNX.

    PYTHON=.venv/bin/python NODE_MODULES=./node_modules EVNX=target/debug/evnx \\
        python3 scripts/measure-dialects.py
"""

import json
import os
import pathlib
import subprocess
import sys
import tempfile

ROOT = pathlib.Path(__file__).resolve().parent.parent
TABLE = ROOT / "src" / "assets" / "dialects.json"

PY = os.environ.get("PYTHON", "python3")
NODE_MODULES = os.environ.get("NODE_MODULES", "node_modules")
EVNX = os.environ.get("EVNX", str(ROOT / "target" / "debug" / "evnx"))

NODE = """
const fs = require('fs'), path = require('path');
const dotenv = require(path.join(process.argv[2], 'dotenv'));
const { expand } = require(path.join(process.argv[2], 'dotenv-expand'));
const plain = dotenv.parse(fs.readFileSync(process.argv[1]));
const expanded = expand({ parsed: { ...plain }, processEnv: {} }).parsed;
console.log(JSON.stringify({ plain, expanded }));
"""


def run(args, **kw):
    r = subprocess.run(args, capture_output=True, text=True, **kw)
    if r.returncode:
        sys.exit(f"{' '.join(args[:3])} failed: {r.stderr.strip()}")
    return r.stdout


def measure(fixture: str, tmp: pathlib.Path) -> dict:
    env = tmp / "case.env"
    env.write_text(fixture, encoding="utf-8")
    out = {}
    out["evnx"] = json.loads(run([EVNX, "convert", "--env", str(env), "--to", "json"]))
    out["python-dotenv"] = json.loads(
        run([PY, "-c", "import json,sys; from dotenv import dotenv_values; "
                       "print(json.dumps(dict(dotenv_values(sys.argv[1]))))", str(env)])
    )
    node = json.loads(run(["node", "-e", NODE, str(env), os.path.abspath(NODE_MODULES)]))
    out["node-dotenv"], out["node-dotenv-expand"] = node["plain"], node["expanded"]
    (tmp / "compose.yaml").write_text("services:\n  s:\n    image: scratch\n    env_file: case.env\n")
    cfg = json.loads(
        run(["docker", "compose", "--project-directory", str(tmp), "-f", str(tmp / "compose.yaml"),
             "config", "--format", "json"], env={**os.environ, "COMPOSE_PROJECT_NAME": "dialects"})
    )
    out["compose"] = cfg["services"]["s"].get("environment", {})
    return out


def main() -> int:
    table = json.loads(TABLE.read_text(encoding="utf-8"))
    drift = 0
    for case in table["cases"]:
        with tempfile.TemporaryDirectory() as d:
            measured = measure(case["fixture"], pathlib.Path(d))
        for reader, expected in case["values"].items():
            got = measured[reader].get(case["key"])
            mark = "ok" if got == expected else "DRIFT"
            drift += got != expected
            print(f"{mark:5} {case['id']:14} {reader:19} {got!r}"
                  + ("" if got == expected else f"  (table: {expected!r})"))
    if drift:
        print(f"\n{drift} value(s) differ from src/assets/dialects.json. Update the table "
              "from these measurements (and its loader versions), never by hand.")
    return 1 if drift else 0


if __name__ == "__main__":
    sys.exit(main())
