#!/usr/bin/env bash
# Puts a self-signed macOS code-signing identity into a keychain.
#
#   self-signed-identity.sh NAME                      make one in the login keychain
#   self-signed-identity.sh NAME KEYCHAIN PASSWORD    make one in KEYCHAIN, unlocked by PASSWORD
#   self-signed-identity.sh --p12 FILE FILE_PASSWORD KEYCHAIN PASSWORD
#                                                     import FILE, made earlier, into KEYCHAIN
#
# A build signed with the same identity each time stays the same app to the
# Keychain, so "Always Allow" for its saved passwords lasts. The certificate
# isn't trusted by anyone else: macOS still asks before the first launch of
# a downloaded copy, as it does for an ad-hoc signature.
set -euo pipefail

# The system's LibreSSL writes a .p12 every `security import` reads; a newer
# OpenSSL needs -legacy for that.
openssl=/usr/bin/openssl
[ -x "$openssl" ] || openssl=openssl
# Not an array: the bash 3.2 macOS ships calls an empty one unset under -u.
legacy=
if "$openssl" version | grep -q '^OpenSSL [3-9]'; then
  legacy=-legacy
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

import() { # FILE FILE_PASSWORD KEYCHAIN [KEYCHAIN_PASSWORD]
  security import "$1" -k "$3" -P "$2" -T /usr/bin/codesign
  if [ -n "${4:-}" ]; then
    # Lets codesign use the key without a dialog, which CI can't answer.
    security set-key-partition-list -S apple-tool:,apple: -s -k "$4" "$3" >/dev/null
  fi
}

if [ "${1:-}" = --p12 ]; then
  [ $# = 5 ] || { echo "usage: $0 --p12 FILE FILE_PASSWORD KEYCHAIN PASSWORD" >&2; exit 2; }
  import "$2" "$3" "$4" "$5"
  exit 0
fi

name=${1:?usage: $0 NAME [KEYCHAIN PASSWORD]}
keychain=${2:-$(security default-keychain -d user | tr -d ' "')}
keychain_password=${3:-}

cat > "$work/cert.conf" <<EOF
[req]
distinguished_name = dn
x509_extensions = ext
prompt = no
[dn]
CN = $name
[ext]
basicConstraints = critical, CA:false
keyUsage = critical, digitalSignature
extendedKeyUsage = critical, codeSigning
EOF
"$openssl" req -x509 -newkey rsa:2048 -nodes -days 3650 -config "$work/cert.conf" \
  -keyout "$work/key.pem" -out "$work/cert.pem" 2>/dev/null
p12_password=$(uuidgen)
"$openssl" pkcs12 -export $legacy -name "$name" -inkey "$work/key.pem" -in "$work/cert.pem" \
  -out "$work/identity.p12" -passout "pass:$p12_password"
import "$work/identity.p12" "$p12_password" "$keychain" "$keychain_password"
echo "Made the code-signing identity \"$name\" in $keychain"
