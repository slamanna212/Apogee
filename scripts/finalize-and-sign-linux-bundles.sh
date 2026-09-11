#!/usr/bin/env bash
# Finalize and sign the Linux updater artifacts, in the order issue #67's
# fix requires:
#
#   compile -> stage AppDir -> apply host-library policy -> build final
#   AppImage -> sign final bytes -> generate updater metadata -> verify
#   -> upload
#
# The caller must have already run an unsigned build (`tauri build
# --no-sign`, e.g. via `tauri-apps/tauri-action` with `args: '--no-sign'`)
# so that nothing under src-tauri/target/release/bundle has been signed
# yet. This script:
#   1. Repacks the AppImage to drop bundled host-interface libraries
#      (scripts/fix-appimage-host-libs.sh) - this is the last modification
#      the AppImage's bytes will ever receive.
#   2. Signs the finalized AppImage, and the deb/rpm bundles (which were
#      also built unsigned by --no-sign and need no repack), using the
#      same Tauri updater signing tool cargo-tauri itself uses
#      (`tauri signer sign`), writing the standard `<file>.sig` sibling
#      next to each.
#
# The caller is then expected to upload these exact files/signatures
# (e.g. via a second tauri-action invocation with `args: '--no-bundle'`,
# which finds and uploads the already-built, already-signed files without
# rebuilding or re-signing them) and never modify them again.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# Overridable only for local/test verification of this script itself; CI
# never sets this and always uses the real cargo-tauri bundle output.
BUNDLE_DIR="${BUNDLE_DIR_OVERRIDE:-$REPO_ROOT/src-tauri/target/release/bundle}"

APPIMAGE="$(ls "$BUNDLE_DIR"/appimage/*.AppImage)"
DEB="$(ls "$BUNDLE_DIR"/deb/*.deb)"
RPM="$(ls "$BUNDLE_DIR"/rpm/*.rpm)"

for f in "$APPIMAGE" "$DEB" "$RPM"; do
  if [ -f "${f}.sig" ]; then
    echo "finalize-and-sign-linux-bundles: $f is already signed (${f}.sig exists)." >&2
    echo "  This script must run on an unsigned build (pass --no-sign to tauri build)." >&2
    exit 1
  fi
done

echo "finalize-and-sign-linux-bundles: applying host-library policy to $(basename "$APPIMAGE")"
"$REPO_ROOT/scripts/fix-appimage-host-libs.sh" "$APPIMAGE"
FINAL_APPIMAGE_SHA256="$(sha256sum "$APPIMAGE" | cut -d' ' -f1)"

if [ -z "${TAURI_SIGNING_PRIVATE_KEY:-}" ]; then
  echo "finalize-and-sign-linux-bundles: TAURI_SIGNING_PRIVATE_KEY is not set" >&2
  exit 1
fi

sign() {
  local path="$1"
  echo "finalize-and-sign-linux-bundles: signing $(basename "$path")"
  (cd "$REPO_ROOT" && npx tauri signer sign "$path")
  [ -f "${path}.sig" ] || {
    echo "finalize-and-sign-linux-bundles: signing $path did not produce ${path}.sig" >&2
    exit 1
  }
}

# Sign the AppImage only after it has been finalized above, and the deb/rpm
# bundles (unmodified since the unsigned build produced them) exactly once
# each, matching cargo-tauri's own updater-signing behavior for these
# formats.
sign "$APPIMAGE"
sign "$DEB"
sign "$RPM"

SIGNED_APPIMAGE_SHA256="$(sha256sum "$APPIMAGE" | cut -d' ' -f1)"
if [ "$SIGNED_APPIMAGE_SHA256" != "$FINAL_APPIMAGE_SHA256" ]; then
  echo "finalize-and-sign-linux-bundles: signer modified the finalized AppImage" >&2
  exit 1
fi

CHECKSUMS="$BUNDLE_DIR/SHA256SUMS-linux-x86_64.txt"
(
  cd "$BUNDLE_DIR"
  sha256sum \
    "${APPIMAGE#"$BUNDLE_DIR"/}" \
    "${DEB#"$BUNDLE_DIR"/}" \
    "${RPM#"$BUNDLE_DIR"/}"
) > "$CHECKSUMS"

echo "finalize-and-sign-linux-bundles: done"
echo "  $APPIMAGE"
echo "  $DEB"
echo "  $RPM"
echo "  $CHECKSUMS"
