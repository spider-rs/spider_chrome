#!/bin/sh
# Release script for the chromey workspace.
#
# Subcrates (types, pdl, cdp, fetcher) track their own version trains
# and only publish when their code changes. Main `chromey` always
# publishes last because it path-deps on all four. We attempt every
# subcrate publish — already-published versions return non-zero from
# cargo, which `|| true` swallows so an unbumped subcrate doesn't
# block the main publish.
set -e

VERSION=$(grep '^version' Cargo.toml | head -1 | sed 's/version = "\(.*\)"/\1/')
if [ -z "$VERSION" ]; then
  echo "ERROR: could not read version from Cargo.toml" >&2
  exit 1
fi
echo "Publishing chromey workspace; main crate at v${VERSION}"

# Subcrate publish in dependency order. types and pdl have no internal
# deps; cdp depends on both; fetcher is independent. Each may already
# exist on crates.io at its current version, in which case cargo
# returns non-zero — `|| true` makes that a no-op.
( cd types   && cargo publish ) || true
( cd pdl     && cargo publish ) || true
( cd cdp     && cargo publish ) || true
( cd fetcher && cargo publish ) || true

# Main crate publishes last. --no-verify because a freshly published
# subcrate may not yet be indexed by crates.io when the main crate's
# verify step tries to download it; this is the standard workaround
# for multi-crate workspaces.
cargo publish --no-verify

echo "Published chromey v${VERSION}"
