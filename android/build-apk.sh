#!/bin/sh
# Builds a signed release APK into ../dist/DREAD.apk.
#
# Environment:
#   JAVA_HOME, ANDROID_HOME     JDK 17+ and Android SDK (platform 36, build-tools 36)
#   DREAD_KEYSTORE              release keystore (default: ./dread-release.jks,
#                               created on first run with a local self-signed key)
#   DREAD_KEYSTORE_PASSWORD, DREAD_KEY_ALIAS, DREAD_KEY_PASSWORD
#
# Keep the keystore: Android only installs updates signed with the same key.
set -eu
cd "$(dirname "$0")"
KS="${DREAD_KEYSTORE:-dread-release.jks}"
if [ ! -f "$KS" ]; then
    echo "== creating local release key $KS"
    "${JAVA_HOME:+$JAVA_HOME/bin/}keytool" -genkeypair -v -keystore "$KS" -alias "${DREAD_KEY_ALIAS:-dread}" \
        -keyalg EC -groupname secp256r1 -validity 10000 \
        -storepass "${DREAD_KEYSTORE_PASSWORD:-dread-local}" -keypass "${DREAD_KEY_PASSWORD:-dread-local}" \
        -dname "CN=DREAD, O=DREAD"
fi
[ -f local.properties ] || echo "sdk.dir=${ANDROID_HOME:?set ANDROID_HOME}" > local.properties
./gradlew --console=plain assembleRelease
mkdir -p ../dist
cp app/build/outputs/apk/release/app-release.apk ../dist/DREAD.apk
echo "built $(cd ../dist && pwd)/DREAD.apk"
