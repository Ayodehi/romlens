#!/usr/bin/env python3
"""Lists every crate in a Cargo.lock as Flatpak sources, so the build needs no
network: each crate is downloaded by flatpak-builder (and checked against the
lockfile's checksum), then Cargo is pointed at them as vendored sources.

    scripts/cargo-sources.py shells/linux/Cargo.lock -o cargo-sources.json

The crates unpack to cargo/vendor/<name>-<version> under CARGO_HOME, which the
manifest sets to /run/build/romlens/cargo. Only crates.io crates are handled:
a git dependency fails here rather than being fetched at build time.
"""
import argparse
import json
import sys
import tomllib

REGISTRY = "registry+https://github.com/rust-lang/crates.io-index"


def sources(lock):
    out = []
    for p in sorted(lock["package"], key=lambda p: (p["name"], p["version"])):
        source = p.get("source")
        if source is None:
            continue  # a path dependency: it is in the source tree
        if source != REGISTRY:
            sys.exit(f"{p['name']} {p['version']}: unsupported source {source}")
        name, version, checksum = p["name"], p["version"], p["checksum"]
        dest = f"cargo/vendor/{name}-{version}"
        out.append({
            "type": "archive",
            "archive-type": "tar-gzip",
            "url": f"https://static.crates.io/crates/{name}/{name}-{version}.crate",
            "sha256": checksum,
            "dest": dest,
        })
        out.append({
            "type": "inline",
            "contents": json.dumps({"package": checksum, "files": {}}),
            "dest": dest,
            "dest-filename": ".cargo-checksum.json",
        })
    out.append({
        "type": "inline",
        "contents": (
            '[source.crates-io]\nreplace-with = "vendored-sources"\n\n'
            '[source.vendored-sources]\ndirectory = "cargo/vendor"\n'
        ),
        "dest": "cargo",
        "dest-filename": "config",
    })
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("lockfile")
    ap.add_argument("-o", "--output", required=True)
    args = ap.parse_args()
    with open(args.lockfile, "rb") as f:
        lock = tomllib.load(f)
    with open(args.output, "w") as f:
        json.dump(sources(lock), f, indent=2)
        f.write("\n")


if __name__ == "__main__":
    main()
