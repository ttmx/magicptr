#!/bin/sh
# Build the magicptr program for the TV, helper included.
# Needs cargo with the armv7-unknown-linux-musleabihf target, javac and the Android SDK.
set -eu
cd "$(dirname "$0")"
helper/build.sh
cargo build --release
