#!/bin/sh
set -e

cd "$(dirname "$0")"
pkill -x dither 2>/dev/null || true
exec cargo run -p dither
