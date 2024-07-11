#!/bin/bash

set -eEuxo pipefail

export TX5_CACHE_DIRECTORY=/data/local/tmp/

trap 'cleanup' ERR EXIT
cleanup() {
  for i in $(cat output-test-executables); do
    adb shell rm -f /data/local/tmp/$(basename $i)
  done
}

for i in $(cat output-test-executables); do
  adb push $i /data/local/tmp/$(basename $i)
  adb shell chmod 500 /data/local/tmp/$(basename $i)
  adb shell RUST_LOG=error RUST_BACKTRACE=1 /data/local/tmp/$(basename $i) --test-threads 1 --nocapture
  adb shell rm -f /data/local/tmp/$(basename $i)
done
