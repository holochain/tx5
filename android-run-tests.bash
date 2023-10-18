#!/bin/bash

set -eEuxo pipefail

for i in $(cat output-test-executables); do
  adb push $i /data/local/tmp/$(basename $i)
  adb shell RUST_BACKTRACE=1 /data/local/tmp/$(basename $i) --test-threads 1 --nocapture
done
