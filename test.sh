#!/bin/bash
set -euo pipefail

test_binary=$(cargo test --no-run --message-format json | jq -r '.executable | select( . != null )')

exec valgrind --leak-check=full $test_binary
