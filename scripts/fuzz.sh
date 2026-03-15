#!/usr/bin/env bash
# Seed-controlled fuzz runner for slopwrap property tests.
# Usage: ./scripts/fuzz.sh [runs] [timeout_secs]
#
# Runs property tests with random seeds via nix, collecting all failures.
# Reproduce any failure with: SLOPWRAP_TEST_SEED=<seed> nix build .#checks.x86_64-linux.unit --rebuild

RUNS=${1:-20}
TIMEOUT=${2:-120}
SYSTEM=$(nix eval --raw --impure --expr builtins.currentSystem)

failed=0
failed_seeds=()

for i in $(seq 1 "$RUNS"); do
    seed=$(($(date +%s%N) % 1000000))
    echo "=== Run $i/$RUNS (seed=$seed) ==="

    if timeout "$TIMEOUT" nix build ".#checks.${SYSTEM}.unit" \
        --rebuild \
        --override-input nixpkgs nixpkgs \
        --argstr SLOPWRAP_TEST_SEED "$seed" 2>&1; then
        echo "  PASS"
    else
        echo "  FAIL (seed=$seed)"
        failed=$((failed + 1))
        failed_seeds+=("$seed")
    fi
done

echo ""
echo "=== Summary: $((RUNS - failed))/$RUNS passed ==="

if [ "$failed" -gt 0 ]; then
    echo "Failed seeds:"
    for s in "${failed_seeds[@]}"; do
        echo "  SLOPWRAP_TEST_SEED=$s nix build .#checks.${SYSTEM}.unit --rebuild"
    done
    exit 1
fi
