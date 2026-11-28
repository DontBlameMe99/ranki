# Android APK build

`android/` packages the `ranki` cdylib into an APK. AndroidX `GameActivity`
loads `libranki.so` and calls the `android_main` symbol already exported by
`crates/bin/src/mobile`. GameActivity provides the soft-keyboard input support
that `NativeActivity` lacks. There is no custom Java or Kotlin source.

The build has two steps:

1. `cargo-ndk` compiles `-p ranki --no-default-features --lib` for
   `aarch64-linux-android` with API 26 and copies the library into
   `android/app/src/main/jniLibs/arm64-v8a/`.
2. The Gradle wrapper runs `assembleDebug` or `assembleRelease` with Android Gradle
   Plugin 8.13.2 and SDK platform 36.

## Prerequisites

- Rust with the `aarch64-linux-android` target. `nix develop` provides it.
- `cargo-ndk` 4.1.2, either from `nix shell nixpkgs#cargo-ndk` or from
  `cargo install cargo-ndk --version 4.1.2 --locked`.
- JDK 17, Android SDK platform 36, build-tools 36, and an NDK. NDK r27 is known
  to work. `ANDROID_HOME` or `ANDROID_SDK_ROOT` must point at the SDK. The script
  selects the newest NDK under `$ANDROID_HOME/ndk`; `ANDROID_NDK_HOME` overrides
  that choice.
- Network access on the first Gradle run, to fetch Gradle 8.14.8 and AGP from
  Google Maven and Maven Central.

`minSdkVersion` is 26. Vulkan needs API 24 or later, and the NDK graphics
bindings used by wgpu 25 expose the AHardwareBuffer path from API 26. Only
`arm64-v8a` is packaged, and the app needs a device with Vulkan or GLES 3.x.

## Commands

From the repository root:

```bash
nix develop --command bash -lc \
  'nix shell nixpkgs#cargo-ndk --command android/scripts/build-apk.sh debug'
```

```bash
nix develop --command bash -lc \
  'nix shell nixpkgs#cargo-ndk --command android/scripts/build-apk.sh release'
```

The script prints the APK path when it succeeds:

- Debug: `android/app/build/outputs/apk/debug/app-debug.apk`, signed with the SDK
  debug key.
- Release: `android/app/build/outputs/apk/release/app-release-unsigned.apk`.
  Release builds are unsigned by default. Add a `signingConfig` with your own
  keystore in `android/app/build.gradle` only when you need a distributable APK.

Generated libraries, Gradle caches, and APKs are ignored by
`android/.gitignore`.

## Script checks

The build script logic can be tested without an SDK, NDK, Rust, or Gradle. It
uses stub commands in a temporary project to check variant selection, NDK
resolution, argument order, output paths, and error handling:

```bash
bash android/scripts/test-build-apk.sh
```
