#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TEST_DIR="$SCRIPT_DIR/testsuite"

if [[ ! -d "$TEST_DIR" ]]; then
  echo "testsuite directory not found: $TEST_DIR" >&2
  exit 1
fi

mapfile -t tests < <(find "$TEST_DIR" -maxdepth 1 -type f -name "*.sh" | sort)

if [[ ${#tests[@]} -eq 0 ]]; then
  echo "No tests found in $TEST_DIR"
  exit 0
fi

echo "Running ${#tests[@]} real-world test(s)"

failures=0
for test_script in "${tests[@]}"; do
  name="$(basename "$test_script")"
  echo "=== RUN $name ==="
  if bash "$test_script"; then
    echo "=== PASS $name ==="
  else
    echo "=== FAIL $name ==="
    failures=$((failures + 1))
  fi
done

if [[ "$failures" -ne 0 ]]; then
  echo "$failures test(s) failed" >&2
  exit 1
fi

echo "All tests passed"
