#!/bin/bash

for i in $(cat output-test-executables); do
  adb push $i /data/local/tmp/$(basename $i)
  adb shell /data/local/tmp/$(basename $i)
done
