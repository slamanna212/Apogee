#!/usr/bin/env bash
# Strip host-interface shared libraries out of a built AppImage.
#
# Tauri's AppImage bundling runs linuxdeploy, which bundles *every* shared
# library the binary links against - including libraries that must come
# from the host system, not the AppDir, because they're the client half of
# a host<->driver ABI. libwayland-client.so.0 is the case that actually
# bites us: the host Mesa is built against the host's Wayland, and if a
# different libwayland-client.so.0 ships inside the AppImage it takes
# precedence (AppRun puts $APPDIR/usr/lib first on LD_LIBRARY_PATH) and
# talks a mismatched ABI to that host Mesa. On rolling-release distros
# (CachyOS/Arch) that run a much newer Wayland/Mesa than the build image,
# this makes the AppImage fail to start. See:
# https://github.com/AppImageCommunity/pkg2appimage/blob/master/excludelist
#
# linuxdeploy has no config knob for this (and Tauri's AppImage bundler
# doesn't expose one either), so this repacks the AppImage after the fact:
# extract -> delete the excluded libs if present -> rebuild with
# appimagetool, in place.
set -euo pipefail

APPIMAGE="${1:?usage: fix-appimage-host-libs.sh <path-to.AppImage>}"
[ -f "$APPIMAGE" ] || { echo "fix-appimage-host-libs: no such file: $APPIMAGE" >&2; exit 1; }
APPIMAGE="$(cd "$(dirname "$APPIMAGE")" && pwd)/$(basename "$APPIMAGE")"

# Host-interface libraries known to break when bundled instead of taken
# from the host. Only libwayland-client.so.0 has been confirmed to cause
# real failures for this app (#67); it's the only one we know is actually
# present in our bundle. Deletion below is a no-op for any name not found,
# so this list can grow without risk to builds where a given lib isn't
# present.
EXCLUDED_LIBS=(
  libwayland-client.so.0
)

WORK_DIR="$(mktemp -d)"
cleanup() { rm -rf "$WORK_DIR"; }
trap cleanup EXIT

TOOLS_DIR="${RUNNER_TEMP:-$WORK_DIR}/appimage-tools"
mkdir -p "$TOOLS_DIR"
APPIMAGETOOL="$TOOLS_DIR/appimagetool-x86_64.AppImage"
if [ ! -x "$APPIMAGETOOL" ]; then
  wget -q -4 -O "$APPIMAGETOOL" \
    https://github.com/AppImage/AppImageKit/releases/download/continuous/appimagetool-x86_64.AppImage
  chmod +x "$APPIMAGETOOL"
fi

cd "$WORK_DIR"
cp "$APPIMAGE" ./input.AppImage
chmod +x ./input.AppImage
./input.AppImage --appimage-extract >/dev/null

REMOVED=()
for lib in "${EXCLUDED_LIBS[@]}"; do
  match="$(find squashfs-root -name "$lib" -print -quit)"
  if [ -n "$match" ]; then
    rm -f "$match"
    REMOVED+=("$lib")
  fi
done

if [ "${#REMOVED[@]}" -eq 0 ]; then
  echo "fix-appimage-host-libs: none of the excluded libraries were bundled, nothing to do"
  exit 0
fi

echo "fix-appimage-host-libs: removed from AppDir: ${REMOVED[*]}"

# appimagetool needs FUSE or --appimage-extract-and-run to run itself in a
# container-y CI environment.
ARCH=x86_64 "$APPIMAGETOOL" --appimage-extract-and-run squashfs-root "$(basename "$APPIMAGE")" >/dev/null
chmod +x "$(basename "$APPIMAGE")"
cp "$(basename "$APPIMAGE")" "$APPIMAGE"
