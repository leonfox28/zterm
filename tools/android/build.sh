#!/bin/sh
set -eu

repo_root=$(CDPATH='' cd -- "$(dirname -- "$0")/../.." && pwd)
# SDK/JDK remain machine configuration, never checked-in Gradle paths.
if [ -z "${ANDROID_HOME:-}" ] && [ -n "${ANDROID_SDK_ROOT:-}" ]; then
    ANDROID_HOME=$ANDROID_SDK_ROOT
    export ANDROID_HOME
fi
if [ "$(uname -s)" = Darwin ]; then
    if [ -z "${ANDROID_HOME:-}" ] && [ -d "$HOME/Library/Android/sdk" ]; then
        ANDROID_HOME="$HOME/Library/Android/sdk"
        export ANDROID_HOME
    fi
    if [ -z "${JAVA_HOME:-}" ]; then
        JAVA_HOME=$(/usr/libexec/java_home -v 17)
        export JAVA_HOME
    fi
fi
if [ "$#" -eq 0 ]; then
    set -- :app:assembleDebug
fi
exec "$repo_root/apps/android/gradlew" -p "$repo_root/apps/android" --console=plain "$@"
