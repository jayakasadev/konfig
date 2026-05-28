#!/usr/bin/env bash
# tools/coverage.sh — Run Rust unit test coverage and produce LCOV reports.
#
# Usage: tools/coverage.sh [output_file]
#   output_file  Path for the LCOV report. Defaults to /tmp/rust_coverage.info.
#
# Requires: bazel, lcov
set -euo pipefail

OUTPUT="${1:-/tmp/rust_coverage.info}"

# Hermetic LLVM from toolchains_llvm — avoids system llvm-cov issues.
HERMETIC_LLVM="$(bazel info output_base)/external/toolchains_llvm++llvm+llvm_toolchain_llvm/bin"

bazel coverage \
  --combined_report=lcov \
  --experimental_generate_llvm_lcov \
  --repo_env=GCOV="${HERMETIC_LLVM}/llvm-profdata" \
  --repo_env=BAZEL_LLVM_COV="${HERMETIC_LLVM}/llvm-cov" \
  --repo_env=BAZEL_LLVM_PROFDATA="${HERMETIC_LLVM}/llvm-profdata" \
  --test_env=COVERAGE_GCOV_PATH="${HERMETIC_LLVM}/llvm-profdata" \
  --test_env=LLVM_COV="${HERMETIC_LLVM}/llvm-cov" \
  --test_tag_filters=unit,-integration,-benchmark \
  //rust/konfig:test

COMBINED="$(bazel info output_path)/_coverage/_coverage_report.dat"
if [[ ! -f "$COMBINED" ]]; then
  echo "ERROR: coverage report not found at $COMBINED" >&2
  exit 1
fi

# Extract only rust/ files.
lcov --extract "$COMBINED" 'rust/*' --output-file "$OUTPUT" 2>/dev/null || cp "$COMBINED" "$OUTPUT"

# Summary.
awk -F: '/^LH/{lh+=$2}/^LF/{lf+=$2}END{if(lf>0) printf "Coverage: %d/%d lines = %.2f%%\n", lh, lf, lh*100.0/lf}' "$OUTPUT"
