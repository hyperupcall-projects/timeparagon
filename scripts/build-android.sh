#!/bin/sh
# Build the TimeSlay Dioxus day view as an Android APK.
#
# The debug APK is debug-signed and debuggable. A --release APK is signed with
# your own keystore and is checked to be non-debuggable.
# You can install it with `adb install -r <apk>` or pass --install.
#
# Usage:
#   scripts/build-android.sh [options]
#
# Options:
#   --abi NAME        arm64 (default), armv7, x86_64, x86
#   --release         Build the release profile, signed with your keystore
#                     (see Environment). Fails without one.
#   --output DIR      Copy the APK into DIR (default: <repo>/dist)
#   --install         Install the APK and launch it on the connected device
#   --help            Show this message
#
# No config or credentials are embedded in the APK. Enter the Google OAuth
# client on the device the first time you open it.
#
# Requirements:
#   - Rust Android targets (the script installs them with rustup if needed)
#   - Android SDK with platform-tools, platforms;android-34,
#     build-tools;34.0.0 and an NDK
#   - `dx` from Dioxus CLI 0.7:
#       cargo binstall dioxus-cli@0.7.10
#   - Java 17 or newer (JAVA_HOME is detected when possible)
#
# Environment:
#   ANDROID_HOME / ANDROID_SDK_ROOT   SDK location
#   ANDROID_NDK_HOME / NDK_HOME       NDK location
#   JAVA_HOME                         JDK location
#   For --release (keep the keystore outside the repository):
#   TIME_SLAY_KEYSTORE              Path to the .jks/.keystore file
#   TIME_SLAY_KEYSTORE_PASSWORD     Keystore password
#   TIME_SLAY_KEY_ALIAS             Key alias
#   TIME_SLAY_KEY_PASSWORD          Key password (default: keystore password)
#   Passwords must not contain the characters " ' ` $ \ & < > =

set -eu
umask 077

script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH='' cd -- "$script_dir/.." && pwd)

# A release build temporarily adds signing settings to Dioxus.toml. Serialize
# builds so another invocation cannot read those settings or restore over them.
exec 9>"$repo_root/target-android-build.lock"
flock -n 9 || { echo "Another Android build is running" >&2; exit 1; }

abi=arm64
profile=debug
install=0
output_dir=$repo_root/dist

usage() {
    sed -n '2,/^$/p' "$0" | sed 's/^# \{0,1\}//'
}

fail() {
    echo "build-android.sh: $*" >&2
    exit 1
}

while [ $# -gt 0 ]; do
    case $1 in
        --abi)
            [ $# -ge 2 ] || fail "--abi needs a value"
            abi=$2
            shift 2
            ;;
        --abi=*)
            abi=${1#--abi=}
            shift
            ;;
        --release)
            profile=release
            shift
            ;;
        --debug)
            profile=debug
            shift
            ;;
        --output)
            [ $# -ge 2 ] || fail "--output needs a directory"
            output_dir=$2
            shift 2
            ;;
        --output=*)
            output_dir=${1#--output=}
            shift
            ;;
        --install)
            install=1
            shift
            ;;
        --help|-h)
            usage
            exit 0
            ;;
        *)
            fail "unknown option: $1 (try --help)"
            ;;
    esac
done

case $abi in
    arm64|arm64-v8a)
        abi_name=arm64-v8a
        triple=aarch64-linux-android
        ;;
    armv7|armeabi-v7a)
        abi_name=armeabi-v7a
        triple=armv7-linux-androideabi
        ;;
    x86_64|x86-64|emulator)
        abi_name=x86_64
        triple=x86_64-linux-android
        ;;
    x86|i686)
        abi_name=x86
        triple=i686-linux-android
        ;;
    *)
        fail "unsupported ABI: $abi (use arm64, armv7, x86_64 or x86)"
        ;;
esac

command -v dx >/dev/null 2>&1 || fail \
    "dx was not found. Install it with: cargo binstall dioxus-cli@0.7.10"

# --- Android SDK -----------------------------------------------------------
sdk=
for candidate in "${ANDROID_HOME:-}" "${ANDROID_SDK_ROOT:-}" \
    "$HOME/.local/state/Android/Sdk" "$HOME/Android/Sdk" \
    "$HOME/Library/Android/sdk"; do
    if [ -n "$candidate" ] && [ -d "$candidate" ]; then
        sdk=$candidate
        break
    fi
done
[ -n "$sdk" ] || fail \
    "Android SDK not found. Set ANDROID_HOME to your SDK directory."

# --- NDK -------------------------------------------------------------------
ndk=${ANDROID_NDK_HOME:-${NDK_HOME:-}}
if [ -z "$ndk" ] && [ -d "$sdk/ndk" ]; then
    ndk=$(find "$sdk/ndk" -mindepth 1 -maxdepth 1 -type d | sort -V | tail -n 1)
fi
[ -n "$ndk" ] && [ -d "$ndk" ] || fail \
    "Android NDK not found. Set ANDROID_NDK_HOME or install an NDK with sdkmanager."

# --- Java ------------------------------------------------------------------
if [ -z "${JAVA_HOME:-}" ] && [ "$profile" = release ] && \
    [ -d "$HOME/.local/share/mise/installs/java/temurin-21.0" ]; then
    JAVA_HOME=$HOME/.local/share/mise/installs/java/temurin-21.0
fi
if [ -z "${JAVA_HOME:-}" ] && command -v mise >/dev/null 2>&1; then
    candidate=$(mise where java 2>/dev/null || true)
    if [ -n "$candidate" ] && [ -d "$candidate" ]; then
        JAVA_HOME=$candidate
    fi
fi
if [ -z "${JAVA_HOME:-}" ]; then
    for candidate in \
        "$HOME/.local/share/mise/installs/java/temurin-21.0" \
        /usr/lib/jvm/java-21-openjdk-amd64 \
        /usr/lib/jvm/java-17-openjdk-amd64 \
        /usr/lib/jvm/default-java; do
        if [ -d "$candidate" ]; then
            JAVA_HOME=$candidate
            break
        fi
    done
fi
if [ -n "${JAVA_HOME:-}" ]; then
    export JAVA_HOME
else
    echo "build-android.sh: warning: JAVA_HOME is not set; using java from PATH" >&2
fi

# The Android Gradle plugin's release lint crashes on JDK 24 and newer.
if [ "$profile" = release ]; then
    java_bin=java
    [ -n "${JAVA_HOME:-}" ] && java_bin=$JAVA_HOME/bin/java
    java_major=$("$java_bin" -version 2>&1 | sed -n '1s/[^"]*"\([0-9]*\).*/\1/p')
    if [ -n "$java_major" ] && [ "$java_major" -ge 24 ]; then
        fail "--release needs JDK 17 to 21 (found $java_major); set JAVA_HOME to one"
    fi
fi

export ANDROID_HOME="$sdk"
export ANDROID_SDK_ROOT="$sdk"
export ANDROID_NDK_HOME="$ndk"
export NDK_HOME="$ndk"
PATH=$sdk/platform-tools:$sdk/emulator:$PATH
export PATH

echo "SDK:  $sdk"
echo "NDK:  $ndk"
echo "ABI:  $abi_name ($triple)"

# --- Rust target -----------------------------------------------------------
if command -v rustup >/dev/null 2>&1; then
    if ! rustup target list --installed | grep -qx "$triple"; then
        echo "Installing Rust target $triple"
        rustup target add "$triple"
    fi
fi

# --- Release signing -------------------------------------------------------
# `dx` only runs Gradle's release variant when Dioxus.toml has a
# [bundle.android] signing block, and it does not read secrets from the
# environment. So for --release the block is appended to Dioxus.toml for the
# duration of the build, then the file is restored and the passwords are
# scrubbed from the generated Gradle file.
dioxus_toml=$repo_root/dioxus/Dioxus.toml
toml_backup=
release_cert=

cleanup() {
    [ -z "$release_cert" ] || rm -f "$release_cert"
    if [ -n "$toml_backup" ] && [ -f "$toml_backup" ]; then
        cp -p "$toml_backup" "$dioxus_toml"
        rm -f "$toml_backup"
    fi
    for gradle in "$repo_root"/target/dx/*/release/android/app/app/build.gradle.kts; do
        [ -f "$gradle" ] && sed -i -E \
            's/^( *(storePassword|keyPassword) = )".*"$/\1"REDACTED"/' "$gradle"
    done
    # Gradle daemons can inherit the descriptor; explicitly release the lock.
    flock -u 9
}
trap cleanup EXIT
trap 'exit 130' INT
trap 'exit 143' TERM

if [ "$profile" = release ]; then
    keystore=${TIME_SLAY_KEYSTORE:-}
    keystore_password=${TIME_SLAY_KEYSTORE_PASSWORD:-}
    key_alias=${TIME_SLAY_KEY_ALIAS:-}
    key_password=${TIME_SLAY_KEY_PASSWORD:-$keystore_password}

    [ -n "$keystore" ] && [ -n "$keystore_password" ] && [ -n "$key_alias" ] || fail \
        "--release needs TIME_SLAY_KEYSTORE, TIME_SLAY_KEYSTORE_PASSWORD and TIME_SLAY_KEY_ALIAS (see --help)"
    [ -f "$keystore" ] || fail "keystore not found: $keystore"
    keystore=$(realpath -- "$keystore")
    for value in "$keystore_password" "$key_alias" "$key_password" "$keystore"; do
        case $value in
            *[\"\'\`\$\\\&\<\>=]*) fail "keystore settings must not contain any of: \" ' \` \$ \\ & < > =" ;;
        esac
        [ "$(printf '%s' "$value" | wc -l)" -eq 0 ] || fail "keystore settings must be single-line values"
    done

    # dx resolves the keystore relative to the repository root.
    keystore_relative=$(realpath --relative-to="$repo_root" "$keystore")

    toml_backup=$(mktemp)
    cp -p "$dioxus_toml" "$toml_backup"
    chmod 600 "$dioxus_toml"
    for gradle in "$repo_root"/target/dx/*/release/android/app/app/build.gradle.kts; do
        [ ! -f "$gradle" ] || chmod 600 "$gradle"
    done
    cat >>"$dioxus_toml" <<EOF

[bundle.android]
jks_file = "$keystore_relative"
jks_password = "$keystore_password"
key_alias = "$key_alias"
key_password = "$key_password"
EOF
    echo "Signing: release keystore $keystore (alias $key_alias)"
fi

# --- Build -----------------------------------------------------------------
cd "$repo_root"
if [ "$profile" = release ]; then
    dx build --android --package time-slay-dioxus --target "$triple" --release
else
    dx build --android --package time-slay-dioxus --target "$triple"
fi

# --- Locate and copy the APK ----------------------------------------------
# `dx` names its output directory after the binary target (TimeSlay), not
# after the package, so search every app directory for this profile. Gradle
# keeps each variant in its own folder, so a --release run cannot pick up a
# debug APK left over from an earlier build.
apk=$(find "$repo_root"/target/dx/*/"$profile"/android -type f -name '*.apk' \
    -path "*/outputs/apk/$profile/*" \
    -printf '%T@ %p\n' 2>/dev/null | sort -n | tail -n 1 | cut -d' ' -f2-)
[ -n "$apk" ] || fail \
    "the build finished but no $profile APK was found under target/dx/*/$profile/android"

# --- Check the release APK -------------------------------------------------
if [ "$profile" = release ]; then
    build_tools=$(find "$sdk/build-tools" -mindepth 1 -maxdepth 1 -type d | sort -V | tail -n 1)
    aapt2=$build_tools/aapt2
    apksigner=$build_tools/apksigner
    [ -x "$aapt2" ] && [ -x "$apksigner" ] || fail \
        "aapt2/apksigner not found in $sdk/build-tools; cannot verify the release APK"

    badging=$("$aapt2" dump badging "$apk") || fail "could not inspect the release APK"
    if printf '%s\n' "$badging" | grep -q '^application-debuggable'; then
        fail "the release APK is debuggable; refusing to publish it"
    fi
    "$apksigner" verify --print-certs "$apk" >"$apk.certs" 2>&1 || {
        rm -f "$apk.certs"
        fail "the release APK does not verify"
    }
    if grep -q 'CN=Android Debug' "$apk.certs"; then
        rm -f "$apk.certs"
        fail "the release APK is signed with an Android debug certificate"
    fi
    # Verify the actual signer, rather than accepting any non-debug signature.
    release_cert=$(mktemp)
    "$JAVA_HOME/bin/keytool" -exportcert -keystore "$keystore" -alias "$key_alias" \
        -storepass:env TIME_SLAY_KEYSTORE_PASSWORD -file "$release_cert" >/dev/null 2>&1 || \
        fail "could not read the configured signing certificate"
    expected_digest=$(sha256sum "$release_cert" | cut -d' ' -f1)
    signer_count=$(grep -c '^Signer #[0-9]* certificate SHA-256 digest:' "$apk.certs" || true)
    actual_digest=$(sed -n 's/^Signer #1 certificate SHA-256 digest: //p' "$apk.certs")
    [ "$signer_count" -eq 1 ] && [ "$actual_digest" = "$expected_digest" ] || \
        fail "the APK signer does not match the configured release key"
    rm -f "$apk.certs"
    echo "Verified: release APK is not debuggable and matches the configured signing key"
fi
mkdir -p "$output_dir"
destination=$output_dir/time-slay-$abi_name-$profile.apk
cp "$apk" "$destination"

echo
echo "APK: $destination"
ls -lh "$destination"
sha256sum "$destination" 2>/dev/null || true

if [ "$install" -eq 1 ]; then
    adb=$sdk/platform-tools/adb
    "$adb" install -r "$destination"
    "$adb" shell am start -n dev.timeslay.day/dev.dioxus.main.MainActivity
fi
