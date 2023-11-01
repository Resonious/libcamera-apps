#!/bin/sh

set -e

cross build --release --target aarch64-unknown-linux-gnu
scp target/aarch64-unknown-linux-gnu/release/libeyecam_net.a nigel@192.168.2.102:~/p/libcamera-apps/libs/libeyecam_net.a