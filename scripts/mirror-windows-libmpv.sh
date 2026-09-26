#!/usr/bin/env bash
# Mirror a zhongfly/mpv-winbuild libmpv archive into this repository's
# releases, where the release workflow downloads the Windows libmpv from.
#
# zhongfly/mpv-winbuild prunes its releases after about a month, so a
# pin on the upstream URL stops downloading a few weeks after it is set.
# The copy lives here as a pre-release tagged `libmpv-windows-<upstream
# tag>`, never marked latest. This script fetches the upstream asset,
# checks it against the SHA-256 digest GitHub reports for it, publishes
# the copy (leaving an identical copy that is already there alone), and
# prints the values to pin in .github/workflows/release.yml.
#
# Usage: scripts/mirror-windows-libmpv.sh <upstream-tag> <asset-name>
#   e.g. scripts/mirror-windows-libmpv.sh 2026-09-14-0b7ed670f7 \
#          mpv-dev-lgpl-x86_64-20260914-git-0b7ed670f7.7z
#
# Needs an authenticated `gh` with write access to the mirror repository.
set -euo pipefail

UPSTREAM="zhongfly/mpv-winbuild"
MIRROR="1337raspberry/ramus"

die() {
  echo "error: $*" >&2
  exit 1
}

if [[ $# -ne 2 ]]; then
  echo "usage: $0 <upstream-tag> <asset-name>" >&2
  exit 2
fi
tag="$1"
asset="$2"
mirror_tag="libmpv-windows-${tag}"

digest="$(gh api "repos/${UPSTREAM}/releases/tags/${tag}" \
  --jq ".assets[] | select(.name == \"${asset}\") | .digest")"
[[ -n "${digest}" ]] || die "no asset ${asset} in ${UPSTREAM} release ${tag}"
expected="${digest#sha256:}"

work="$(mktemp -d)"
trap 'rm -rf "${work}"' EXIT
gh release download "${tag}" --repo "${UPSTREAM}" --pattern "${asset}" --dir "${work}"
actual="$(shasum -a 256 "${work}/${asset}" | cut -d' ' -f1)"
[[ "${actual}" == "${expected}" ]] \
  || die "SHA-256 mismatch for ${asset}: GitHub reports ${expected}, download is ${actual}"

notes="Copy of \`${asset}\` from https://github.com/${UPSTREAM}/releases/tag/${tag}, kept here because upstream prunes its releases after about a month. The release workflow downloads it from this release and checks its SHA-256 before bundling \`libmpv-2.dll\` into the Windows installer.

SHA-256: \`${actual}\`

An LGPL build of libmpv and FFmpeg. Source: https://github.com/mpv-player/mpv and https://github.com/FFmpeg/FFmpeg; build recipe: https://github.com/${UPSTREAM}."

if gh release view "${mirror_tag}" --repo "${MIRROR}" >/dev/null 2>&1; then
  existing="$(gh api "repos/${MIRROR}/releases/tags/${mirror_tag}" \
    --jq ".assets[] | select(.name == \"${asset}\") | .digest")"
  if [[ "${existing}" == "sha256:${actual}" ]]; then
    echo "${MIRROR} ${mirror_tag} already holds an identical ${asset}"
  elif [[ -n "${existing}" ]]; then
    die "${MIRROR} ${mirror_tag} holds a different ${asset} (${existing}); delete it first"
  else
    gh release upload "${mirror_tag}" "${work}/${asset}" --repo "${MIRROR}"
  fi
else
  gh release create "${mirror_tag}" "${work}/${asset}" \
    --repo "${MIRROR}" \
    --target main \
    --prerelease \
    --latest=false \
    --title "libmpv for Windows (${tag})" \
    --notes "${notes}"
fi

cat <<EOF

Pin these in the "Download libmpv DLL" step of .github/workflows/release.yml:
          MPV_RELEASE_TAG: ${tag}
          MPV_ASSET: ${asset}
          MPV_ASSET_SHA256: ${actual}
EOF
