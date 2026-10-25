#!/bin/bash
# Local replication of .github/workflows/ci.yml — the four CI steps,
# exactly, before pushing. Run this until every step reports OK; only then
# push (GitHub Actions credits are not a test bench).
#
# Resource guardrails for small machines: nice, -j 2 on the test step,
# orphaned rustc/lld killed between steps. On a beefier machine you can
# drop the nice and the job cap.
#
# Usage: scripts/ci-local.sh            (streams to stdout; ~30-60 min cold)
set -u
cd "$(dirname "$0")/.." || exit 1
mkdir -p target/tmp
export TMPDIR="$PWD/target/tmp"
export CARGO_TERM_COLOR=always

step() { echo; echo "=== $1 ==="; }
kill_orphans() {
  pkill -f "[r]ustc" 2>/dev/null
  pkill -f "[r]ust-lld" 2>/dev/null
  sleep 1
}

step "CI 1/4: cargo build --workspace --all-targets"
nice -n 19 cargo build --workspace --all-targets && echo "BUILD-OK" || echo "BUILD-FAIL"
kill_orphans

step "CI 2/4: TMPDIR=\$PWD/target/tmp cargo test --workspace --jobs 2"
nice -n 19 cargo test --workspace --jobs 2 2>&1 | grep -E "test result:|FAILED|panicked at|error\[|^error" | head -80
echo "TESTS-DONE (scan the lines above for FAILED / non-ok results)"
kill_orphans

step "CI 3/4: clippy -D warnings"
nice -n 19 cargo clippy --workspace --all-targets -- -D warnings && echo "CLIPPY-OK" || echo "CLIPPY-FAIL"
kill_orphans

step "CI 4/4: format check"
nice -n 19 cargo fmt --all -- --check && echo "RUSTFMT-OK" || echo "RUSTFMT-FAIL"
nice -n 19 cargo run -q -p rex-cli -- fmt --check \
  examples/*.mox examples/*.actor examples/*.ifml \
  tests/conformance/models/*.mox tests/conformance/models/*.actor \
  tests/conformance/ddd/*.ddd tests/conformance/sigil/*.mox \
  tests/conformance/sigil/*.actor tests/conformance/sigil/*.ddd \
  tests/conformance/events/*.mox tests/conformance/events/*.evt \
  tests/conformance/deploy/*.deploy tests/conformance/usecases/*.uc \
  patterns/ifml/*.ifml tests/conformance/ifml/app.ifml \
  && echo "REXFMT-OK" || echo "REXFMT-FAIL"

echo
echo "ALL FOUR STEPS MUST END IN *-OK BEFORE PUSHING."
echo "CI-REPLICATION-DONE"
