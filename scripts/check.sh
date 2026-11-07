#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/.."
python3 scripts/check-tools.py
actionlint
cargo fmt --all --check
cargo check --locked --all-targets --all-features
cargo clippy --locked --all-targets --all-features -- -D warnings
python3 scripts/check-test-suite.py
cargo deny --locked check
typos
python3 scripts/test_app_identity.py
python3 scripts/test_desktop_integration.py
python3 scripts/test_picker_integration.py
python3 -m unittest discover -s scripts -p 'release_test*.py' -v
