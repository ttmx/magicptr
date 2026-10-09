#!/bin/sh
# Build magicptr.dex, the helper the daemon installs and runs on the box. Needs javac and the Android SDK.
set -eu
SDK=${ANDROID_HOME:-/opt/android-sdk}
PLATFORM=$(ls -d "$SDK"/platforms/android-* | sort -V | tail -1)
D8=$(command -v d8 || ls "$SDK"/build-tools/*/d8 | sort -V | tail -1)
cd "$(dirname "$0")"
tmp=$(mktemp -d)
javac -source 8 -target 8 -Xlint:-options -cp "$PLATFORM/android.jar" -d "$tmp" MagicPtr.java
"$D8" --min-api 21 --lib "$PLATFORM/android.jar" --output "$tmp" "$tmp"/*.class
mv "$tmp/classes.dex" magicptr.dex
rm -r "$tmp"
