#!/usr/bin/env bash
# Makes a self-signed macOS code-signing identity in the login keychain, for
# `make sign-identity`.
#
#   self-signed-identity.sh NAME
#
# A local build signed with the same identity each time stays the same app
# to the Keychain, so "Always Allow" for its saved passwords lasts. Release
# builds from CI are signed ad hoc instead.
set -euo pipefail

name=${1:?usage: $0 NAME}

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
password=$(uuidgen)
"$openssl" pkcs12 -export $legacy -name "$name" -inkey "$work/key.pem" -in "$work/cert.pem" \
  -out "$work/identity.p12" -passout "pass:$password"
keychain=$(security default-keychain -d user | tr -d ' "')
security import "$work/identity.p12" -k "$keychain" -P "$password" -T /usr/bin/codesign
echo "Made the code-signing identity \"$name\" in $keychain"
