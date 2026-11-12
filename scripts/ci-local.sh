#!/bin/bash
# Local replication of .github/workflows/ci.yml — the five CI steps,
# exactly, before pushing. Run this until every step reports *-OK; only
# then push (GitHub Actions credits are not a test bench).
#
# Mirrors CI's env: CARGO_INCREMENTAL=0 and RUSTFLAGS="-C debuginfo=0"
# (the dsntk disk-budget tweak, workflow env block), and the Tests step
# runs under TMPDIR=$PWD/target/tmp with --jobs 2 (the doctest SIGBUS
# remedy — the AGENTS.md testing-quirks gotcha). Small-machine guardrails:
# nice, orphaned rustc/lld killed between steps. On a beefier machine you
# can drop the nice and the job cap.
#
# The rexlang fmt --check file list below is copied VERBATIM from the
# Format check step of .github/workflows/ci.yml — keep the two identical.
#
# Usage: scripts/ci-local.sh            (summary lines stream; ~30-60 min cold)
set -u
cd "$(dirname "$0")/.." || exit 1
mkdir -p target/tmp
export TMPDIR="$PWD/target/tmp"
export CARGO_TERM_COLOR=always
export CARGO_INCREMENTAL=0
export RUSTFLAGS="-C debuginfo=0"

step() { echo; echo "=== $1 ==="; }
kill_orphans() {
  pkill -f "[r]ustc" 2>/dev/null
  pkill -f "[r]ust-lld" 2>/dev/null
  sleep 1
}

step "CI 1/5: cargo build --workspace --all-targets"
nice -n 19 cargo build --workspace --all-targets && echo "BUILD-OK" || echo "BUILD-FAIL"
kill_orphans

step "CI 2/5: TMPDIR=\$PWD/target/tmp cargo test --workspace --jobs 2"
test_log=target/tmp/ci-local-tests.log
if nice -n 19 cargo test --workspace --jobs 2 > "$test_log" 2>&1; then
  grep "test result:" "$test_log" | tail -20
  echo "TESTS-OK"
else
  grep -E "test result:|FAILED|panicked at|error\[|^error" "$test_log" | head -80
  echo "TESTS-FAIL (full output: $test_log)"
fi
kill_orphans

step "CI 3/5: clippy -D warnings"
nice -n 19 cargo clippy --workspace --all-targets -- -D warnings && echo "CLIPPY-OK" || echo "CLIPPY-FAIL"
kill_orphans

step "CI 4/5: format check"
nice -n 19 cargo fmt --all -- --check && echo "RUSTFMT-OK" || echo "RUSTFMT-FAIL"
nice -n 19 cargo run -q -p rex-cli -- fmt --check \
  examples/*.mox examples/*.actor examples/*.ifml examples/*.evt examples/*.dmn examples/*.ddd examples/*.uc examples/*.deploy examples/*.cmmn tests/conformance/models/*.mox tests/conformance/models/*.actor tests/conformance/ddd/*.ddd tests/conformance/sigil/*.mox tests/conformance/sigil/*.actor tests/conformance/sigil/*.ddd tests/conformance/events/*.mox tests/conformance/events/*.evt tests/conformance/deploy/*.deploy tests/conformance/usecases/*.uc tests/conformance/cmmn/*.cmmn tests/conformance/dmn/*.dmn patterns/ifml/*.ifml tests/conformance/ifml/app.ifml tests/conformance/ifml/cli.ifml tests/conformance/ifml/rexlang/auth.mox tests/conformance/ifml/rexlang/auth.actor \
  && echo "REXFMT-OK" || echo "REXFMT-FAIL"
kill_orphans

# The strict rustdoc gate: broken intra-doc links are warnings by default;
# -D warnings makes them errors (issue #175). cargo doc runs no doctests,
# but it inherits the script's TMPDIR export like every step.
step "CI 5/5: RUSTDOCFLAGS=-D warnings cargo doc --workspace --no-deps"
nice -n 19 env RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps \
  && echo "DOC-OK" || echo "DOC-FAIL"
kill_orphans

echo
echo "ALL FIVE STEPS MUST END IN *-OK BEFORE PUSHING."
echo "CI-LOCAL-DONE"
