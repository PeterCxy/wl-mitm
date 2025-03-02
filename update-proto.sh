#!/usr/bin/env bash
set -e

TEMP="$(mktemp -d)"
PROJECT_DIR="${PWD}"

pushd "$TEMP"

cleanup() {
    rm -rf "$TEMP"
}

trap cleanup EXIT

rm -rf "${PROJECT_DIR}"/proto/*

git clone --recursive --depth=1 https://github.com/vially/wayland-explorer
for xml in $(find "wayland-explorer/protocols/" -name '*.xml'); do
    if [[ "$xml" =~ "test"* ]]; then
        continue
    fi

    if grep -q -E "^<protocol name=(.*)>$" "$xml"; then
        echo "Found Wayland protocol definition $xml"
        cp "$xml" "${PROJECT_DIR}"/proto
    fi
done

for deprecated_proto in $(grep -Po "(?<=')([^/]*)\.xml(?=')" wayland-explorer/scripts/bin/regenerate-protocols-data.ts); do
    if [ -f "${PROJECT_DIR}"/proto/"$deprecated_proto" ]; then
        rm "${PROJECT_DIR}"/proto/"$deprecated_proto"
    fi
done
