#!/usr/bin/env bash
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$HERE/../.." && pwd)"

mkdir -p "$REPO_ROOT/.tmp"
TMP="$(mktemp -p "$REPO_ROOT/.tmp" -d)"
trap 'rm -rf "$TMP"' EXIT

fail() {
  printf 'FAIL: %s\n' "$*" >&2
  exit 1
}

contains() {
  grep -Fq -- "$2" "$1"
}

assert_contains() {
  contains "$1" "$2" || fail "expected to find: $2"
}

assert_not_contains() {
  if contains "$1" "$2"; then
    fail "expected not to find: $2"
  fi
}

SDK="$TMP/sdk"
PROJECT="$TMP/project"
mkdir -p "$SDK/ndk/25.2.9519653" "$SDK/ndk/27.1.12297006" "$TMP/bin"
mkdir -p "$PROJECT/android/scripts" "$PROJECT/android/app/src/main"
cp "$HERE/build-apk.sh" "$PROJECT/android/scripts/build-apk.sh"
chmod +x "$PROJECT/android/scripts/build-apk.sh"
: > "$PROJECT/Cargo.toml"

cat > "$TMP/bin/cargo" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
printf 'NDK=%s\n' "${ANDROID_NDK_HOME:-}" >> "$CARGO_LOG"
printf 'CARGO=%s\n' "$*" >> "$CARGO_LOG"
if [[ "${1:-}" == ndk && "${2:-}" == --version ]]; then
  echo "cargo-ndk 4.1.2"
  exit 0
fi
output=""
previous=""
for arg in "$@"; do
  if [[ "$previous" == "--output-dir" || "$previous" == "-o" ]]; then
    output="$arg"
  fi
  previous="$arg"
done
[[ -n "$output" ]] || exit 1
[[ ! -e "$output/arm64-v8a/libranki_mobile.so" ]] || exit 1
mkdir -p "$output/arm64-v8a"
: > "$output/arm64-v8a/libranki_mobile.so"
STUB

cat > "$PROJECT/android/gradlew" <<'STUB'
#!/usr/bin/env bash
set -euo pipefail
printf 'GRADLE=%s\n' "$*" >> "$GRADLE_LOG"
project=""
task=""
while (( $# > 0 )); do
  case "$1" in
    -p) project="$2"; shift 2 ;;
    assembleDebug|assembleRelease) task="$1"; shift ;;
    *) shift ;;
  esac
done
[[ -n "$project" && -n "$task" ]] || exit 1
case "$task" in
  assembleDebug) variant=debug; artifact=app-debug.apk ;;
  assembleRelease) variant=release; artifact=app-release-unsigned.apk ;;
esac
mkdir -p "$project/app/build/outputs/apk/$variant"
: > "$project/app/build/outputs/apk/$variant/$artifact"
STUB
chmod +x "$TMP/bin/cargo" "$PROJECT/android/gradlew"

SCRIPT="$PROJECT/android/scripts/build-apk.sh"
export CARGO="$TMP/bin/cargo"
export CARGO_LOG="$TMP/cargo.log"
export GRADLE_LOG="$TMP/gradle.log"
export ANDROID_HOME="$SDK"
unset ANDROID_SDK_ROOT ANDROID_NDK_HOME NDK_HOME

: > "$CARGO_LOG"
: > "$GRADLE_LOG"
"$SCRIPT" debug > "$TMP/output.log"
assert_contains "$CARGO_LOG" "NDK=$SDK/ndk/27.1.12297006"
assert_contains "$CARGO_LOG" "CARGO=ndk --target arm64-v8a --platform 26 --output-dir $PROJECT/android/app/src/main/jniLibs build --manifest-path $PROJECT/Cargo.toml --package ranki-mobile --lib"
assert_not_contains "$CARGO_LOG" "CARGO=ndk --target arm64-v8a --platform 26 --output-dir $PROJECT/android/app/src/main/jniLibs build --manifest-path $PROJECT/Cargo.toml --package ranki-mobile --lib --release"
assert_contains "$GRADLE_LOG" "GRADLE=-p $PROJECT/android --no-daemon assembleDebug"
assert_contains "$TMP/output.log" "APK: $PROJECT/android/app/build/outputs/apk/debug/app-debug.apk"

: > "$CARGO_LOG"
: > "$GRADLE_LOG"
"$SCRIPT" release > "$TMP/output.log"
assert_contains "$CARGO_LOG" "CARGO=ndk --target arm64-v8a --platform 26 --output-dir $PROJECT/android/app/src/main/jniLibs build --manifest-path $PROJECT/Cargo.toml --package ranki-mobile --lib --release"
assert_contains "$GRADLE_LOG" "GRADLE=-p $PROJECT/android --no-daemon assembleRelease"
assert_contains "$TMP/output.log" "APK: $PROJECT/android/app/build/outputs/apk/release/app-release-unsigned.apk"

: > "$GRADLE_LOG"
if "$SCRIPT" staging > /dev/null 2>&1; then
  fail "invalid variant was accepted"
fi
if "$SCRIPT" debug extra > /dev/null 2>&1; then
  fail "extra argument was accepted"
fi
[[ ! -s "$GRADLE_LOG" ]] || fail "invalid invocation reached Gradle"

: > "$CARGO_LOG"
ANDROID_NDK_HOME="$SDK/ndk/25.2.9519653" "$SCRIPT" debug > /dev/null
assert_contains "$CARGO_LOG" "NDK=$SDK/ndk/25.2.9519653"

: > "$CARGO_LOG"
if ANDROID_HOME="$TMP/nope" "$SCRIPT" debug 2> "$TMP/error.log" > /dev/null; then
  fail "missing SDK was accepted"
fi
assert_contains "$TMP/error.log" "ANDROID_HOME"
[[ ! -s "$CARGO_LOG" ]] || fail "cargo ran without an SDK"

printf 'ok: script checks passed\n'
