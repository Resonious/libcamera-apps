#!/bin/sh

set -e

remote=nigel@192.168.2.102

cross build --release --target aarch64-unknown-linux-gnu
scp target/aarch64-unknown-linux-gnu/release/libeyecam_net.a "$remote:~/p/libcamera-apps/libs/libeyecam_net.a"
ssh "$remote" bash -c 'cd ~/p/libcamera-apps/build && meson compile'
