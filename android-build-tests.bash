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

export PATH="$NDK_ROOT/bin:$PATH"
export PKG_CONFIG_SYSROOT_DIR="${NDK_ROOT}/sysroot"
export TARGET_CC="${NDK_ROOT}/bin/${ANDROID_ARCH}-linux-android${ANDROID_API_LEVEL}-clang"
export TARGET_CFLAGS="-I${NDK_ROOT}/sysroot/usr/include -I${NDK_ROOT}/sysroot/usr/include/${ANDROID_ARCH}-linux-android"
export TARGET_AR="${NDK_ROOT}/bin/llvm-ar"
export TARGET_RANLIB="${NDK_ROOT}/bin/llvm-ranlib"
export CGO_CFLAGS="-I${NDK_ROOT}/sysroot/usr/include -I${NDK_ROOT}/sysroot/usr/include/${ANDROID_ARCH}-linux-android"

# https://github.com/aws/aws-lc-rs/issues/751
export BINDGEN_EXTRA_CLANG_ARGS_x86_64_linux_android="--sysroot=${NDK_ROOT}/sysroot"
export CFLAGS_x86_64_linux_android="${BINDGEN_EXTRA_CLANG_ARGS_x86_64_linux_android}"

cargo test -p tx5 \
  --no-default-features \
  --features backend-go-pion \
  --no-run \
  --target ${ANDROID_ARCH}-linux-android \
  --config target.${ANDROID_ARCH}-linux-android.linker="\"${NDK_ROOT}/bin/${ANDROID_ARCH}-linux-android34-clang\"" \
  --config target.${ANDROID_ARCH}-linux-android.ar="\"${NDK_ROOT}/bin/llvm-ar\"" \
  2>&1 | tee output-cargo-test

cat output-cargo-test | grep Executable | sed -E 's/[^(]*\(([^)]*)\)/\1/' > output-test-executables
echo "BUILD TESTS:"
cat output-test-executables
