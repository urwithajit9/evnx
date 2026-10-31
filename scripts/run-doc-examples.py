#!/usr/bin/env python3
"""Run every `evnx …` example in the evnx-web guides and say what happened (RM-38).

    python3 scripts/run-doc-examples.py <evnx-bin> <evnx-web-dir> [--only <glob>]

Each ```bash line starting `evnx ` in packages/docs-content/guides/**/*.mdx runs
in a fresh copy of a small seeded project (git repo, .env, .env.example,
.env.production, a template), with no terminal, a throwaway HOME and config
folder, and every evnx URL pointed at a closed port, so nothing reaches
api.evnx.dev or any other host. Each example ends up as one of:

    ran            exit 0 or 1 (1 is a finding: drift, a secret, a failed check)
    needs-context  exit 2 for a reason the seed cannot supply: a file it does
                   not have, a terminal, a server, a sign-in
    error          anything else: a code other than 0/1/2, a panic, a timeout,
                   or exit 2 for a reason not listed above. Read these first.
    skipped        not run: shell syntax (pipes, redirects, $VAR), a <placeholder>,
                   or a command that needs the network (cloud, migrate, update --check)

Prints a table and a count per class. Exits 0 whatever it finds: it is a
report, not a gate, and is not run in CI yet. `--only` limits it to guide
paths matching a glob (`--only 'commands/*'`).
"""

import fnmatch
import os
import pathlib
import re
import shlex
import shutil
import subprocess
import sys
import tempfile

TIMEOUT = 20
CLOSED = "http://127.0.0.1:9"

# Need a server, a sign-in or another host; never run against a seed.
NETWORK = {"auth", "vault", "cloud", "org", "migrate"}

# exit 2 explained by what the seed lacks, not by the example being wrong.
CONTEXT = re.compile(
    r"does not exist|not found|no such file|no terminal|not a terminal|"
    r"not signed in|sign in|connection refused|could not connect|"
    r"error sending request|network|needs network support|nothing was piped|"
    r"left nothing to convert|backup file not found",
    re.I,
)

SEED = {
    ".env": "DATABASE_URL=postgres://app:pw@localhost:5432/app\n"
    "API_KEY=sk_test_CHANGE_ME\nPORT=8080\nDEBUG=false\n",
    ".env.example": "DATABASE_URL=\nAPI_KEY=\nPORT=8080\nDEBUG=false\nNEW_FLAG=\n",
    ".env.production": "DATABASE_URL=postgres://app:pw@db.internal:5432/app\n"
    "API_KEY=\nPORT=80\nDEBUG=false\n",
    ".gitignore": ".env\n.env.production\n",
    "config.yaml.tmpl": "port: {{PORT}}\ndb: {{DATABASE_URL|yaml}}\n",
}


def examples(web):
    root = pathlib.Path(web) / "packages" / "docs-content" / "guides"
    for mdx in sorted(root.rglob("*.mdx")):
        rel = mdx.relative_to(root).as_posix()
        in_bash = False
        pending, start = "", 0
        for n, line in enumerate(mdx.read_text(encoding="utf-8").splitlines(), 1):
            s = line.strip()
            if s.startswith("```"):
                in_bash = not in_bash and s[3:].strip() in ("bash", "sh", "shell")
                pending = ""
                continue
            if not in_bash:
                continue
            if pending:
                pending += " " + s.rstrip("\\").strip()
                if not s.endswith("\\"):
                    yield rel, start, pending
                    pending = ""
                continue
            s = s.removeprefix("$ ")
            if not s.startswith("evnx "):
                continue
            if s.endswith("\\"):
                pending, start = s.rstrip("\\").strip(), n
            else:
                yield rel, n, s


def why_skip(cmd):
    if re.search(r"[|><;&`]|\$\(|\$\{|\$[A-Z_]", cmd):
        return "shell syntax"
    if re.search(r"<[a-z][^>]*>|\.\.\.|…", cmd):
        return "placeholder"
    if re.search(r"\[[A-Z-]", cmd):
        return "usage synopsis"
    try:
        words = shlex.split(cmd, comments=True)
    except ValueError:
        return "does not parse"
    sub = words[1] if len(words) > 1 else ""
    if sub in NETWORK:
        return f"needs the network ({sub})"
    if sub == "update" and "--check" in words:
        return "needs the network (update --check)"
    return None


def seed(dest):
    for name, text in SEED.items():
        (dest / name).write_text(text)
    subprocess.run(["git", "init", "-q"], cwd=dest, check=True)
    subprocess.run(["git", "add", ".env.example", ".gitignore"], cwd=dest, check=True)


def run_one(evnx, cmd, base):
    work = pathlib.Path(tempfile.mkdtemp(dir=base))
    proj, home = work / "project", work / "home"
    proj.mkdir()
    home.mkdir()
    seed(proj)
    env = {
        "PATH": os.environ.get("PATH", ""),
        "HOME": str(home),
        "EVNX_CONFIG_DIR": str(home / "evnx"),
        "EVNX_API_URL": CLOSED,
        "EVNX_SERVER": CLOSED,
        "NO_COLOR": "1",
        "TERM": "dumb",
    }
    words = shlex.split(cmd, comments=True)
    try:
        r = subprocess.run(
            [evnx] + words[1:],
            cwd=proj,
            env=env,
            stdin=subprocess.DEVNULL,
            capture_output=True,
            text=True,
            timeout=TIMEOUT,
        )
    except subprocess.TimeoutExpired:
        return "error", "timeout", f"no answer in {TIMEOUT}s"
    finally:
        shutil.rmtree(work, ignore_errors=True)
    lines = (r.stderr + "\n" + r.stdout).splitlines()
    # The first line that says something: not the `evnx <cmd>  <title>` header.
    first = next(
        (l.strip() for l in lines if l.strip() and not l.strip().startswith("evnx ")), ""
    )
    if r.returncode in (0, 1):
        return "ran", r.returncode, ""
    if " -- " in cmd:
        # `evnx run -- <program>` exits with the program's code.
        return "needs-context", r.returncode, f"the child program: {first}"
    if CONTEXT.search(r.stderr + r.stdout):
        return "needs-context", r.returncode, first
    return "error", r.returncode, first


def main():
    args = sys.argv[1:]
    only = None
    if "--only" in args:
        i = args.index("--only")
        only = args[i + 1]
        del args[i : i + 2]
    if len(args) != 2:
        sys.exit(__doc__)
    evnx, web = os.path.abspath(args[0]), args[1]
    print(subprocess.run([evnx, "--version"], capture_output=True, text=True).stdout.strip())

    rows, counts = [], {"ran": 0, "needs-context": 0, "error": 0, "skipped": 0}
    with tempfile.TemporaryDirectory(prefix="evnx-doc-") as base:
        for rel, line, cmd in examples(web):
            if only and not fnmatch.fnmatch(rel, only):
                continue
            reason = why_skip(cmd)
            if reason:
                result, code, note = "skipped", "", reason
            else:
                result, code, note = run_one(evnx, cmd, base)
            counts[result] += 1
            rows.append((result, f"{rel}:{line}", code, cmd, note))

    order = {"error": 0, "needs-context": 1, "ran": 2, "skipped": 3}
    for result, where, code, cmd, note in sorted(rows, key=lambda r: (order[r[0]], r[1])):
        print(f"{result:13} {str(code):4} {where:45} {cmd}")
        if note and result != "ran":
            print(f"{'':19}{note[:150]}")
    print()
    print("  ".join(f"{k}: {v}" for k, v in counts.items()), f" total: {len(rows)}")


if __name__ == "__main__":
    main()
