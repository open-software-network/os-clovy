#!/usr/bin/env python3
"""Fail unless package.json pins a pnpm release at least seven days old."""

import argparse
import datetime
import json
import re
import sys
import time
import urllib.request

REGISTRY_URL = "https://registry.npmjs.org/pnpm"
USER_AGENT = (
    "clovy-pnpm-release-age-check "
    "(https://github.com/guilhermexp/os-clovy)"
)


def pinned_version(package_json_path):
    with open(package_json_path) as package_file:
        package_json = json.load(package_file)
    package_manager = package_json.get("packageManager", "")
    match = re.fullmatch(r"pnpm@([^+\s]+)(?:\+sha\d+\..+)?", package_manager)
    if not match:
        raise ValueError(
            "package.json#packageManager must pin an exact pnpm version"
        )
    version = match.group(1)
    if not re.fullmatch(r"\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?", version):
        raise ValueError(f"pnpm version must be exact, got {version!r}")
    return version


def fetch_packument(attempts=3):
    last_error = None
    for attempt in range(attempts):
        if attempt:
            time.sleep(2**attempt)
        try:
            request = urllib.request.Request(
                REGISTRY_URL,
                headers={"User-Agent": USER_AGENT},
            )
            with urllib.request.urlopen(request, timeout=30) as response:
                return json.load(response)
        except Exception as error:  # noqa: BLE001 - retry every transport error
            last_error = error
    raise RuntimeError(f"npm registry lookup failed: {last_error}")


def publication_time(packument, version):
    published = packument.get("time", {}).get(version)
    if not published:
        raise ValueError(f"pnpm@{version} has no publication time in npm metadata")
    return datetime.datetime.fromisoformat(published.replace("Z", "+00:00"))


def release_age_error(version, published, now, min_age_days):
    age = now - published
    minimum_age = datetime.timedelta(days=min_age_days)
    if age < minimum_age:
        return (
            f"pnpm@{version} was published {age.days}d "
            f"{age.seconds // 3600}h ago (< {min_age_days}d cooldown)"
        )
    return None


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--package-json", default="package.json")
    parser.add_argument("--min-age-days", type=int, default=7)
    args = parser.parse_args()

    try:
        version = pinned_version(args.package_json)
        published = publication_time(fetch_packument(), version)
        now = datetime.datetime.now(datetime.timezone.utc)
        error = release_age_error(version, published, now, args.min_age_days)
    except (OSError, ValueError, RuntimeError, json.JSONDecodeError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1

    if error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    age = now - published
    print(
        f"pnpm release-age check passed: pnpm@{version} is "
        f"{age.days}d {age.seconds // 3600}h old"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
