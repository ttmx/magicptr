#!/bin/sh
# Package the TV app (settings screen plus the magicptr program) as an .ipk for the Homebrew
# Channel, with the manifest its store needs. Run ./build.sh first. Needs npx.
set -eu
cd "$(dirname "$0")"
ID=io.github.ttmx.magicptr
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml)
REPO=https://github.com/ttmx/magicptr

rm -rf dist && mkdir -p dist/stage/bin
cp app/appinfo.json app/index.html app/app.js app/style.css app/icon.png app/largeIcon.png dist/stage/
sed -i "s/\"version\": \".*\"/\"version\": \"$VERSION\"/" dist/stage/appinfo.json
cp target/armv7-unknown-linux-musleabihf/release/magicptr dist/stage/bin/
npx --yes -p @webos-tools/cli ares-package --no-minify -o dist dist/stage
rm -r dist/stage

IPK=${ID}_${VERSION}_all.ipk
cat > dist/$ID.manifest.json <<JSON
{
    "id": "$ID",
    "version": "$VERSION",
    "type": "web",
    "title": "Magic Pointer",
    "appDescription": "Point and click on an Android TV box with the Magic Remote",
    "iconUri": "$REPO/raw/main/app/largeIcon.png",
    "sourceUrl": "$REPO",
    "rootRequired": true,
    "ipkUrl": "$IPK",
    "ipkHash": {
        "sha256": "$(sha256sum dist/$IPK | cut -d' ' -f1)"
    }
}
JSON
ls -l dist
