#!/bin/bash
# Run the Kani proof harnesses in checkers-core.
#
# Kani does not build on Windows (it hard-depends on std::os::unix), so this must
# run under Linux, WSL, or CI. From a Windows shell:
#
#   wsl.exe -d Debian bash /mnt/c/.../scripts/verify-proofs.sh
set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if ! command -v cargo-kani >/dev/null 2>&1; then
    echo "cargo-kani not found. Install with:"
    echo "  cargo install kani-verifier && kani setup"
    exit 127
fi

# Kani cannot write into a Windows-mounted target dir reliably, and reuses the
# host target dir otherwise; keep its artifacts separate.
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/tmp/kani-target-checkers}"

echo "Running Kani proofs in $REPO/checkers-core"
echo "  target dir: $CARGO_TARGET_DIR"
echo

cd "$REPO" || exit 1
cargo kani -p checkers-core "$@" 2>&1 | tee /tmp/kani-checkers.log

echo
echo "===== summary ====="
grep -E "^Checking harness|VERIFICATION:- (SUCCESSFUL|FAILED)" /tmp/kani-checkers.log \
    | paste - - 2>/dev/null \
    | sed 's/Checking harness //; s/\.\.\.//'

failed=$(grep -c "VERIFICATION:- FAILED" /tmp/kani-checkers.log)
passed=$(grep -c "VERIFICATION:- SUCCESSFUL" /tmp/kani-checkers.log)
echo
echo "proofs verified: $passed, failed: $failed"

if [ "$failed" -ne 0 ]; then
    echo
    echo "===== failures ====="
    grep -B3 -A8 "VERIFICATION:- FAILED" /tmp/kani-checkers.log
    exit 1
fi

[ "$passed" -eq 0 ] && { echo "no proofs ran"; exit 1; }
exit 0
