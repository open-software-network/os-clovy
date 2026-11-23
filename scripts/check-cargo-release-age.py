#!/usr/bin/env python3
"""Cargo release-age cooldown: refuse crate versions younger than 7 days.

Cargo has no minimumReleaseAge equivalent (rust-lang/cargo#15973), so this
script fills the gap. It diffs every tracked Cargo.lock (discovered via git,
current and base) against a base ref and, for every crates.io version that
is new or bumped, queries crates.io for the publish date. Versions published
less than --min-age-days ago fail the check (exit 1). Network or metadata
errors fail closed after retries — a cooldown that silently passes is not a
gate.

Escape hatch for an urgent patch inside the window: add `name@version` to
scripts/cargo-release-age-exclude.txt with a trailing `# reason` comment.

Runs on stock python3 (3.9+), stdlib only.
Usage: python3 scripts/check-cargo-release-age.py [--base origin/main]
"""

import argparse
import datetime
import json
import re
import subprocess
import sys
import time
import urllib.request

EXCLUDE_FILE = "scripts/cargo-release-age-exclude.txt"
CRATES_IO_REGISTRY = "registry+https://github.com/rust-lang/crates.io-index"
USER_AGENT = "clovy-release-age-check (https://github.com/guilhermexp/os-clovy)"
API = "https://crates.io/api/v1/crates/{name}/versions"


def parse_lock(text):
    """(name, version) pairs for crates.io-sourced packages in a Cargo.lock."""
    packages = set()
    name = version = source = None
    for line in text.splitlines() + ["[[package]]"]:
        line = line.strip()
        if line == "[[package]]":
            if name and version and source == CRATES_IO_REGISTRY:
                packages.add((name, version))
            name = version = source = None
        elif m := re.match(r'^name = "(.+)"$', line):
            name = m.group(1)
        elif m := re.match(r'^version = "(.+)"$', line):
            version = m.group(1)
        elif m := re.match(r'^source = "(.+)"$', line):
            source = m.group(1)
    return packages


def base_lock(base, path):
    p = subprocess.run(
        ["git", "show", f"{base}:{path}"], capture_output=True, text=True
    )
    return p.stdout if p.returncode == 0 else ""


def discover_lockfiles(base):
    """Every Cargo.lock tracked now or at the base ref, paired across renames.

    Discovered dynamically so a new Rust tree cannot slip past a hardcoded
    list. True deletions remain in the result and fail closed, while a detected
    rename compares the current lockfile with its previous path at the base.
    """
    current = subprocess.run(
        ["git", "ls-files", "*Cargo.lock"], capture_output=True, text=True
    ).stdout.splitlines()
    at_base = subprocess.run(
        ["git", "ls-tree", "-r", "--name-only", base],
        capture_output=True,
        text=True,
    ).stdout.splitlines()
    current = {path for path in current if path}
    at_base = {path for path in at_base if path.endswith("Cargo.lock")}
    changes = subprocess.run(
        ["git", "diff", "--name-status", "--find-renames", base, "--"],
        capture_output=True,
        text=True,
    ).stdout.splitlines()
    renamed_to_base = {}
    for change in changes:
        fields = change.split("\t")
        if (
            len(fields) == 3
            and fields[0].startswith("R")
            and fields[1].endswith("Cargo.lock")
            and fields[2].endswith("Cargo.lock")
        ):
            renamed_to_base[fields[2]] = fields[1]

    renamed_sources = set(renamed_to_base.values())
    lockfiles = {(path, renamed_to_base.get(path, path)) for path in current}
    lockfiles |= {
        (path, path)
        for path in at_base
        if path not in current and path not in renamed_sources
    }
    if not lockfiles:
        raise RuntimeError("no Cargo.lock files found in the repo or at base")
    return sorted(lockfiles)


def lockfile_tree_retired(lockfile, manifests):
    """Whether a deleted lockfile's whole Rust tree is gone.

    A lockfile only guards the manifest beside it. When that Cargo.toml is no
    longer tracked, no build can resolve crates from the tree, so the deletion
    needs no check; while the manifest remains, losing its lock fails closed.
    """
    parent = lockfile.rsplit("/", 1)[0] if "/" in lockfile else ""
    manifest = f"{parent}/Cargo.toml" if parent else "Cargo.toml"
    return manifest not in manifests


def discover_manifests():
    """Every tracked Cargo manifest, including new standalone Rust trees."""
    result = subprocess.run(
        ["git", "ls-files", "*Cargo.toml"], capture_output=True, text=True
    )
    if result.returncode != 0:
        raise RuntimeError(f"failed to discover Cargo.toml files: {result.stderr}")
    manifests = sorted(path for path in result.stdout.splitlines() if path)
    if not manifests:
        raise RuntimeError("no Cargo.toml files found in the repo")
    return manifests


def verify_manifests_locked(manifests=None):
    """Fail when any tracked manifest would resolve without a committed lock."""
    manifests = manifests or discover_manifests()
    for manifest in manifests:
        result = subprocess.run(
            [
                "cargo",
                "metadata",
                "--locked",
                "--format-version",
                "1",
                "--manifest-path",
                manifest,
            ],
            capture_output=True,
            text=True,
        )
        if result.returncode != 0:
            detail = result.stderr.strip() or result.stdout.strip()
            raise RuntimeError(f"{manifest} is not locked: {detail}")
    return len(manifests)


def load_exclusions():
    try:
        lines = open(EXCLUDE_FILE).read().splitlines()
    except FileNotFoundError:
        return set()
    return {
        line.split("#")[0].strip()
        for line in lines
        if line.split("#")[0].strip()
    }


def published_at(name, version, attempts=3):
    url = API.format(name=name)
    last_err = None
    for attempt in range(attempts):
        if attempt:
            time.sleep(2**attempt)
        try:
            req = urllib.request.Request(url, headers={"User-Agent": USER_AGENT})
            with urllib.request.urlopen(req, timeout=30) as resp:
                data = json.load(resp)
            for v in data.get("versions", []):
                if v.get("num") == version:
                    created = v["created_at"].replace("Z", "+00:00")
                    return datetime.datetime.fromisoformat(created)
            raise LookupError(f"{name}@{version} not found on crates.io")
        except LookupError:
            raise
        except Exception as err:  # noqa: BLE001 — retry any transport error
            last_err = err
    raise RuntimeError(f"crates.io lookup failed for {name}: {last_err}")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--base", default="origin/main")
    parser.add_argument("--min-age-days", type=int, default=7)
    args = parser.parse_args()

    if (
        subprocess.run(
            ["git", "rev-parse", "--verify", f"{args.base}^{{commit}}"],
            capture_output=True,
        ).returncode
        != 0
    ):
        print(f"error: base ref {args.base!r} does not resolve", file=sys.stderr)
        return 1

    try:
        manifests = discover_manifests()
        manifest_count = verify_manifests_locked(manifests)
        lockfiles = discover_lockfiles(args.base)
    except RuntimeError as err:
        print(f"error: {err}", file=sys.stderr)
        return 1

    excluded = load_exclusions()
    now = datetime.datetime.now(datetime.timezone.utc)
    cutoff = datetime.timedelta(days=args.min_age_days)
    new_packages = set()
    for path, base_path in lockfiles:
        try:
            current = parse_lock(open(path).read())
        except FileNotFoundError:
            # Fail closed: a deleted lockfile would let later builds resolve
            # fresh crates that never saw this check, unless its tree is gone.
            if lockfile_tree_retired(path, manifests):
                print(f"retired: {path} (its Cargo.toml was removed)")
                continue
            print(f"error: {path} missing", file=sys.stderr)
            return 1
        new_packages |= current - parse_lock(base_lock(args.base, base_path))

    violations = []
    checked = 0
    for name, version in sorted(new_packages):
        if f"{name}@{version}" in excluded:
            print(f"excluded: {name}@{version} (see {EXCLUDE_FILE})")
            continue
        if checked:
            time.sleep(1)  # crates.io crawler policy: max 1 request/second
        try:
            age = now - published_at(name, version)
        except (LookupError, RuntimeError) as err:
            violations.append(f"{name}@{version}: {err}")
            continue
        checked += 1
        if age < cutoff:
            violations.append(
                f"{name}@{version} published {age.days}d "
                f"{age.seconds // 3600}h ago (< {args.min_age_days}d cooldown)"
            )

    if violations:
        print(
            f"cargo release-age check failed ({args.min_age_days}d cooldown, "
            f"base {args.base}):",
            file=sys.stderr,
        )
        for v in violations:
            print(f"  {v}", file=sys.stderr)
        print(
            f"For an urgent patch, add `name@version # reason` to "
            f"{EXCLUDE_FILE}.",
            file=sys.stderr,
        )
        return 1
    print(
        f"cargo release-age check passed: {manifest_count} manifest(s) locked, "
        f"{len(new_packages)} new crate "
        f"version(s) vs {args.base}, {checked} checked, "
        f"{len(new_packages) - checked} excluded"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
