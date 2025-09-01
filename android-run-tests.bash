#!/bin/bash

set -eEuxo pipefail

if [[ "${ANDROID_API_LEVEL:-x}" == "x" ]]; then
  echo "ANDROID_API_LEVEL required"
  exit 127
fi

if [[ "${ANDROID_NDK_VERSION:-x}" == "x" ]]; then
  echo "ANDROID_NDK_VERSION required"
  exit 127
fi

if [[ "${ANDROID_ARCH:-x}" == "x" ]]; then
  echo "ANDROID_ARCH required"
  exit 127
else
  if [[ "${ANDROID_ARCH}" == "arm64-v8a" ]]; then
    export ANDROID_ARCH="aarch64"
  fi
fi


if [[ "${ANDROID_SDK_ROOT:-x}" == "x" ]]; then
  echo "ANDROID_SDK_ROOT required"
  exit 127
fi

export ANDROID_NDK_ROOT="$ANDROID_SDK_ROOT/ndk/$ANDROID_NDK_VERSION"

ndk_root_folder=""
case "$(uname -m)" in
  x86_64) ndk_root_folder="linux-x86_64" ;;
  aarch64) ndk_root_folder="linux-aarch64" ;;
  *)
    echo "Unsupported build host arch: $(uname -m)"
    exit 1
    ;;
esac

case "$ANDROID_ARCH" in
  "arm64-v8a")
    export ANDROID_ARCH="aarch64"
    ;;
  "armeabi-v7a")
    export ANDROID_ARCH="arm"
    ;;
  "x86")
    export ANDROID_ARCH="i686"
    ;;
  "x86_64")
    export ANDROID_ARCH="x86_64"
    ;;
  *)
    echo "Unsupported ANDROID_ARCH: $ANDROID_ARCH"
    exit 1
    ;;
esac

NDK_ROOT="${ANDROID_SDK_ROOT}/ndk/${ANDROID_NDK_VERSION}/toolchains/llvm/prebuilt/${ndk_root_folder}"
if [ ! -d "$NDK_ROOT" ]; then
  echo "NDK_ROOT does not exist: $NDK_ROOT"
  exit 1
fi

# get libc++_shared.so path
LIBCPP_SHARED_SO_PATH="${NDK_ROOT}/sysroot/usr/lib/${ANDROID_ARCH}-linux-android/libc++_shared.so"
adb push $LIBCPP_SHARED_SO_PATH /data/local/tmp/

trap 'cleanup' ERR EXIT
cleanup() {
  for i in $(cat output-test-executables); do
    adb shell rm -f /data/local/tmp/$(basename $i)
  done
}

for i in $(cat output-test-executables); do
  adb push $i /data/local/tmp/$(basename $i)
  adb shell chmod 500 /data/local/tmp/$(basename $i)
  adb shell LD_LIBRARY_PATH=/data/local/tmp TX5_CACHE_DIRECTORY=/data/local/tmp/ RUST_LOG=error RUST_BACKTRACE=1 /data/local/tmp/$(basename $i) --test-threads 1 --nocapture
  adb shell rm -f /data/local/tmp/$(basename $i)
done
