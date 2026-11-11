#!/bin/sh
# Builds the app, starts it offscreen, records a still and a film of its window, stops it.
set -eu
recorder="${LOOK_RECORD:-look-record}"
build="$FEARLEZZ_OUT/build"
mkdir -p "$build/LookDemo.app/Contents/MacOS"
cp Info.plist "$build/LookDemo.app/Contents/"
swiftc -O -swift-version 5 DemoApp.swift -o "$build/LookDemo.app/Contents/MacOS/LookDemo"
set -a
. ./look.env
set +a
DEMO_STATIC=1 DEMO_X=-3000 "$build/LookDemo.app/Contents/MacOS/LookDemo" &
app=$!
trap 'kill $app 2>/dev/null' EXIT
sleep 1
"$recorder" still --pid "$app" --out home.png --id home --crop-top 32
"$recorder" film --pid "$app" --out home.mp4 --id home-film --crop-top 32 --seconds 2
