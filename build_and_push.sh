#!/bin/bash
set -eu

cmd="docker"

sed -i -E "s/version = \"[0-9]+\.[0-9]+\.[0-9]+\"/version = \"$1\"/g" Cargo.toml

cargo test

if ! [[ $(command -v $cmd) ]]; then
    cmd="podman"
fi

tag="docker.io/embarkstudios/octobors:$1"

$cmd build -t "$tag" -f Dockerfile .
$cmd push "$tag"
