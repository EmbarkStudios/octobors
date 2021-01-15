#!/bin/bash
set -eu

cmd="docker"

if ! [[ $(command -v $cmd) ]]; then
    cmd="podman"
fi

tag="docker.io/embarkstudios/octobors:$1"

$cmd build -t "$tag" -f Dockerfile .
$cmd push "$tag"

sed -i -E "s/:[0-9]+\.[0-9]+\.[0-9]+/:$1/g" action.dockerfile
