#!/bin/bash

if [[ "x${ANDROID_API_LEVEL}" == "x" ]]; then
  echo "ANDROID_API_LEVEL required"
  exit 127
fi

if [[ "x${ANDROID_NDK_VERSION}" == "x" ]]; then
  echo "ANDROID_NDK_VERSION required"
  exit 127
fi

if [[ "x${ANDROID_ARCH}" == "x" ]]; then
  echo "ANDROID_ARCH required"
  exit 127
fi

if [[ "${ANDROID_ARCH}" == "arm64-v8a" ]]; then
  export ANDROID_ARCH="aarch64"
fi

if [[ "x${ANDROID_SDK_ROOT}" == "x" ]]; then
  echo "ANDROID_SDK_ROOT required"
  exit 127
fi

_ndk_root=(${ANDROID_SDK_ROOT}/ndk/${ANDROID_NDK_VERSION}/toolchains/llvm/prebuilt/*)

cat << EOF > ${_ndk_root}/lib/clang/17/lib/linux/${ANDROID_ARCH}/libgcc.a
INPUT(-lunwind)
EOF

export PKG_CONFIG_SYSROOT_DIR="${_ndk_root}/sysroot"
export CC="${_ndk_root}/bin/${ANDROID_ARCH}-linux-android${ANDROID_API_LEVEL}-clang"
export AR="${_ndk_root}/bin/llvm-ar"
export RANLIB="${_ndk_root}/bin/llvm-ranlib"
export CGO_CFLAGS="-I${_ndk_root}/sysroot/usr/include"

cargo test --no-run --release --target ${ANDROID_ARCH}-linux-android --config target.${ANDROID_ARCH}-linux-android.linker="\"${_ndk_root}/bin/${ANDROID_ARCH}-linux-android34-clang\"" --config target.${ANDROID_ARCH}-linux-android.ar="\"${_ndk_root}/bin/llvm-ar\"" 2>&1 | tee output-cargo-test
cat output-cargo-test | grep Executable | sed -E 's/[^(]*\(([^)]*)\)/\1/' > output-test-executables
echo "BUILD TESTS:"
cat output-test-executables
