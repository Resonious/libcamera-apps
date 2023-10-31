#!/bin/bash

APP="${APP:-/home/nigel/p/libcamera-apps/build/apps/libcamera-vid}"
SECRET="${SECRET:-pi}"

while true; do
  $APP "--webrtc=$SECRET" --timeout=0 --buffer-count=2
done