#!/usr/bin/env bash
#
# scripts/verify-trud-bundle.sh <dir> <releases.json>
#
# Checks a directory holding a source bundle's four files (NHS's zip, its checksum
# trud_<name>.xml, the signature trud_<name>.xml.asc and the key trud-public-key-*.pgp), the
# directory `ods make oci --source` reads and `ods trud pull` fills.
#
#   1. NHS signed it, with a pinned key. The bundled key is only used to check the signature; the
#      trust is the fingerprint pinned in `trud_signing_key_fingerprints` in <releases.json>,
#      normally the repo's data/releases.json. A key that verifies but isn't pinned fails.
#   2. The signed file names this zip.
#   3. The signed hash is this zip's SHA-1.
#
# Prints one ✓ line, or a ✖ block naming the failed check and both values, and exits 1. Lines are
# labelled with the zip's filename, and the ✓ line ends with the zip's SHA-256 (upper case).
# Needs gpg, gpgv, jq, openssl and base64.
#
set -euo pipefail

usage() {
  echo "usage: scripts/verify-trud-bundle.sh <dir> <releases.json>" >&2
  exit 2
}

[[ $# -eq 2 ]] || usage
DIR="$1"
INDEX="$2"
[[ -d "$DIR" ]] || { echo "✖ $DIR isn't a directory" >&2; exit 2; }
[[ -f "$INDEX" ]] || { echo "✖ $INDEX isn't a file" >&2; exit 2; }
for tool in gpg gpgv jq openssl base64; do
  command -v "$tool" >/dev/null 2>&1 || { echo "✖ $tool is needed and isn't installed" >&2; exit 2; }
done

# The one file in $DIR matching a glob, or a ✖ block
find_one() {
  local label="$1" pattern="$2" found=""
  local count=0 f
  for f in "$DIR"/$pattern; do
    [[ -f "$f" ]] || continue
    found="$f"
    count=$((count + 1))
  done
  if [[ "$count" -ne 1 ]]; then
    echo "✖ $DIR  needs exactly one $label ($pattern), found $count" >&2
    exit 1
  fi
  echo "$found"
}

ZIP="$(find_one "zip" '*.zip')"
XML="$(find_one "checksum" 'trud_*.xml')"
SIG="$(find_one "signature" 'trud_*.xml.asc')"
KEY="$(find_one "public key" 'trud-public-key-*.pgp')"

# Lines are labelled with the zip's filename: the date in it is NHS's stamp, which isn't always
# TRUD's release date.
ZIP_NAME="$(basename "$ZIP")"

fail() {
  local check="$1"
  shift
  echo "✖ $ZIP_NAME  $check" >&2
  local line
  for line in "$@"; do
    echo "  $line" >&2
  done
  exit 1
}

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# 1. NHS signed it, with a pinned key
gpg --dearmor <"$KEY" >"$WORK/nhs.gpg" 2>/dev/null || fail "the bundled key can't be read" "$(basename "$KEY")"
STATUS="$(gpgv --status-fd 1 --keyring "$WORK/nhs.gpg" "$SIG" "$XML" 2>/dev/null || true)"
VALIDSIG="$(grep '^\[GNUPG:\] VALIDSIG ' <<<"$STATUS" | head -n 1 || true)"
if [[ -z "$VALIDSIG" ]]; then
  fail "the signature doesn't verify" "gpgv found no valid signature of $(basename "$XML")" "by the key in $(basename "$KEY")"
fi
FINGERPRINT="$(awk '{print $3}' <<<"$VALIDSIG")"
SIGNED_ON="$(awk '{print $4}' <<<"$VALIDSIG")"
PINNED="$(jq -r '.trud_signing_key_fingerprints[]' "$INDEX")"
if ! grep -qx "$FINGERPRINT" <<<"$PINNED"; then
  fail "the signing key isn't pinned" \
    "signed by  $FINGERPRINT" \
    "pinned     $(tr '\n' ' ' <<<"$PINNED")(trud_signing_key_fingerprints in $INDEX)"
fi

# 2. The signed file names this zip
NAMED="$(sed -n 's:.*<name>\(.*\)</name>.*:\1:p' "$XML" | head -n 1)"
if [[ "$NAMED" != "$ZIP_NAME" ]]; then
  fail "NHS's signed checksum names a different zip" "the checksum's <name>  $NAMED" "the zip's filename     $ZIP_NAME"
fi

# 3. The signed hash is this zip's
EXPECTED_SHA1="$(sed -n 's:.*<SHA1>\(.*\)</SHA1>.*:\1:p' "$XML" | head -n 1)"
ACTUAL_SHA1="$(openssl dgst -sha1 -binary "$ZIP" | base64)"
if [[ "$ACTUAL_SHA1" != "$EXPECTED_SHA1" ]]; then
  fail "the zip isn't the one NHS signed" "NHS's SHA-1      $EXPECTED_SHA1" "the zip's SHA-1  $ACTUAL_SHA1"
fi

# The zip's SHA-256, upper case as TRUD and _provenance.json write it. NHS signs only the SHA-1;
# the SHA-256 is TRUD's, and ties this zip to TRUD's listing.
SHA256="$(openssl dgst -sha256 -r "$ZIP" | awk '{print toupper($1)}')"

echo "✓ $ZIP_NAME signed by $FINGERPRINT on $SIGNED_ON; NHS-signed SHA-1 matches; SHA-256 $SHA256"
