#!/usr/bin/env bash
# Finalize a built AppImage by stripping host-interface shared libraries out
# of it, before it is signed.
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
# this makes the AppImage fail to start (issue #67). See:
# https://github.com/AppImageCommunity/pkg2appimage/blob/master/excludelist
#
# The installed Tauri CLI (2.11.4) hardcodes its linuxdeploy invocation
# (crates/tauri-bundler/src/bundle/linux/appimage/linuxdeploy.rs) with no
# `--exclude-library` passthrough and no tauri.conf.json knob for it, and
# linuxdeploy performs AppDir staging and final AppImage packaging in that
# one invocation with no hook in between. So this repacks the AppImage
# after linuxdeploy builds it, but - unlike the workaround this replaced -
# it MUST run before the file is signed: extract -> delete the excluded
# libs if present -> rebuild with appimagetool, in place. The caller is
# responsible for calling this before `tauri signer sign`, not after
# upload; signing bytes this script hasn't already finalized would sign
# the wrong artifact.
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

# Pinned appimagetool release (AppImage/appimagetool, the maintained
# successor to the old AppImageKit "continuous" build this used to pull).
# A numbered release plus a hardcoded checksum means CI never trusts a
# floating tag's current contents. Only x86_64 is pinned because that's
# the only arch this project builds Linux bundles for.
APPIMAGETOOL_VERSION="1.9.1"
APPIMAGETOOL_ARCH="x86_64"
APPIMAGETOOL_URL="https://github.com/AppImage/appimagetool/releases/download/${APPIMAGETOOL_VERSION}/appimagetool-${APPIMAGETOOL_ARCH}.AppImage"
# Verified locally against the GitHub release asset digest for 1.9.1 (see
# https://github.com/AppImage/appimagetool/releases/tag/1.9.1) before
# pinning it here.
APPIMAGETOOL_SHA256="ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0"

# appimagetool also downloads a "runtime" stub from the floating
# AppImage/type2-runtime "continuous" tag at repack time unless told
# otherwise, which would leave this step non-deterministic and dependent on
# a second network fetch. Pin that too, from the dated (non-"continuous")
# 20251108 release, and embed it explicitly via --runtime-file.
RUNTIME_VERSION="20251108"
RUNTIME_ARCH="x86_64"
RUNTIME_URL="https://github.com/AppImage/type2-runtime/releases/download/${RUNTIME_VERSION}/runtime-${RUNTIME_ARCH}"
# Verified locally against the GitHub release asset digest for the
# 20251108 release before pinning it here.
RUNTIME_SHA256="2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d"

WORK_DIR="$(mktemp -d)"
cleanup() { rm -rf "$WORK_DIR"; }
trap cleanup EXIT

verify_checksum() {
  local candidate="$1" expected="$2"
  [ -f "$candidate" ] || return 1
  local actual
  actual="$(sha256sum "$candidate" | cut -d' ' -f1)"
  [ "$actual" = "$expected" ]
}

# Fetches $2 to cache path $1, verifying against expected checksum $3.
# Re-verifies cache hits too, since a checksum mismatch means the cached
# file is not the pinned release and must never be trusted or reused.
fetch_pinned() {
  local dest="$1" url="$2" expected="$3" label="$4"
  if [ -f "$dest" ] && ! verify_checksum "$dest" "$expected"; then
    echo "fix-appimage-host-libs: cached $label failed checksum verification, re-downloading" >&2
    rm -f "$dest"
  fi
  if [ ! -f "$dest" ]; then
    wget -q -4 -O "$dest" "$url"
    if ! verify_checksum "$dest" "$expected"; then
      echo "fix-appimage-host-libs: downloaded $label failed checksum verification" >&2
      echo "  expected: $expected" >&2
      echo "  actual:   $(sha256sum "$dest" | cut -d' ' -f1)" >&2
      rm -f "$dest"
      exit 1
    fi
  fi
}

TOOLS_DIR="${RUNNER_TEMP:-$WORK_DIR}/appimage-tools"
mkdir -p "$TOOLS_DIR"
APPIMAGETOOL="$TOOLS_DIR/appimagetool-${APPIMAGETOOL_ARCH}-${APPIMAGETOOL_VERSION}.AppImage"
RUNTIME_FILE="$TOOLS_DIR/runtime-${RUNTIME_ARCH}-${RUNTIME_VERSION}"

fetch_pinned "$APPIMAGETOOL" "$APPIMAGETOOL_URL" "$APPIMAGETOOL_SHA256" "appimagetool ${APPIMAGETOOL_VERSION}"
chmod +x "$APPIMAGETOOL"
fetch_pinned "$RUNTIME_FILE" "$RUNTIME_URL" "$RUNTIME_SHA256" "type2-runtime ${RUNTIME_VERSION}"

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
# container-y CI environment. --runtime-file pins the embedded runtime stub
# instead of letting appimagetool fetch "continuous" over the network.
ARCH="$APPIMAGETOOL_ARCH" "$APPIMAGETOOL" --appimage-extract-and-run \
  --runtime-file "$RUNTIME_FILE" \
  squashfs-root "$(basename "$APPIMAGE")" >/dev/null
chmod +x "$(basename "$APPIMAGE")"
cp "$(basename "$APPIMAGE")" "$APPIMAGE"
