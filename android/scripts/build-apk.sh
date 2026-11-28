#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: android/scripts/build-apk.sh [debug|release]

Builds the existing ranki cdylib for arm64-v8a with cargo-ndk, then
assembles the APK with Gradle. The default variant is debug.
EOF
}

die() {
  printf 'error: %s\n' "$*" >&2
  exit 1
}

MODE="${1:-debug}"
if (( $# > 1 )); then
  usage >&2
  exit 2
fi
case "$MODE" in
  debug|release) ;;
  -h|--help)
    usage
    exit 0
    ;;
  *)
    usage >&2
    exit 2
    ;;
esac

ANDROID_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPO_ROOT="$(cd "$ANDROID_DIR/.." && pwd)"
CARGO="${CARGO:-cargo}"
GRADLEW="${GRADLEW:-$ANDROID_DIR/gradlew}"

SDK="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}"
[[ -n "$SDK" && -d "$SDK" ]] || die "ANDROID_HOME must point to an installed Android SDK"

if [[ -n "${ANDROID_NDK_HOME:-}" || -n "${NDK_HOME:-}" ]]; then
  NDK="${ANDROID_NDK_HOME:-$NDK_HOME}"
  [[ -d "$NDK" ]] || die "Configured NDK directory does not exist: $NDK"
else
  candidates=()
  if [[ -d "$SDK/ndk-bundle" ]]; then
    candidates+=("$SDK/ndk-bundle")
  fi
  if [[ -d "$SDK/ndk" ]]; then
    while IFS= read -r dir; do
      candidates+=("$dir")
    done < <(find "$SDK/ndk" -mindepth 1 -maxdepth 1 -type d | sort -V)
  fi
  [[ ${#candidates[@]} -gt 0 ]] || die "No Android NDK found under $SDK/ndk. Set ANDROID_NDK_HOME."
  NDK="${candidates[-1]}"
fi
export ANDROID_NDK_HOME="$NDK"

command -v "$CARGO" >/dev/null 2>&1 || die "cargo not found. Run this script inside 'nix develop'."
"$CARGO" ndk --version >/dev/null 2>&1 || die "cargo-ndk not found. Add nixpkgs#cargo-ndk to PATH or run 'cargo install cargo-ndk --version 4.1.2 --locked'."
[[ -x "$GRADLEW" ]] || die "Gradle wrapper not found or not executable: $GRADLEW"

JNI_DIR="$ANDROID_DIR/app/src/main/jniLibs"
SO="$JNI_DIR/arm64-v8a/libranki.so"
rm -f "$SO"
CARGO_ARGS=(
  ndk --target arm64-v8a --platform 26 --output-dir "$JNI_DIR"
  build --manifest-path "$REPO_ROOT/Cargo.toml" --package ranki --no-default-features --lib
)
if [[ "$MODE" == release ]]; then
  CARGO_ARGS+=(--release)
fi

printf 'Rust -> %s\n' "$SO"
"$CARGO" "${CARGO_ARGS[@]}"

[[ -f "$SO" ]] || die "cargo-ndk did not produce $SO"

case "$MODE" in
  debug) GRADLE_TASK=assembleDebug ;;
  release) GRADLE_TASK=assembleRelease ;;
esac
printf 'Gradle -> %s\n' "$GRADLE_TASK"
"$GRADLEW" -p "$ANDROID_DIR" --no-daemon "$GRADLE_TASK"

if [[ "$MODE" == debug ]]; then
  APK="$ANDROID_DIR/app/build/outputs/apk/debug/app-debug.apk"
else
  APK="$ANDROID_DIR/app/build/outputs/apk/release/app-release-unsigned.apk"
  if [[ ! -f "$APK" ]]; then
    APK="$ANDROID_DIR/app/build/outputs/apk/release/app-release.apk"
  fi
fi
[[ -f "$APK" ]] || die "Gradle did not produce an APK for $MODE"
printf 'APK: %s\n' "$APK"
