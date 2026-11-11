#!/usr/bin/env bash
# Make the self-signed certificate Pointcast's macOS builds are signed with. Run it once, on any system with
# openssl, and keep what it writes outside the repository:
#
#   scripts/make-signing-cert.sh /somewhere/safe/pointcast-signing.p12
#
# It writes the .p12 and, beside it, a .password file with the password the .p12 is sealed with. It prints the
# certificate's SHA-1 and the two commands that hand both to GitHub, and nothing secret. docs/SIGNING.md says what
# the certificate is for and what losing it costs.
set -euo pipefail

out=${1:?usage: make-signing-cert.sh <path of the .p12 to write> [common name]}
name=${2:-maxnaid}
repo=${POINTCAST_REPO:-naidx0/pointcast}
stop() {
  echo "$*" >&2
  exit 1
}

[ ! -e "$out" ] && [ ! -e "$out.password" ] ||
  stop "$out is there already. Every installed Pointcast is known to macOS by its certificate: choose another path rather than replace it."
dir=$(cd "$(dirname "$out")" && pwd) || stop "The folder for $out does not exist."
if git -C "$dir" rev-parse --show-toplevel >/dev/null 2>&1; then
  stop "$dir is inside a git repository. The certificate must never be committed: write it somewhere else."
fi

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
umask 077

# Ten years, for signing code and nothing else.
cat >"$work/cert.cnf" <<EOF
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
subjectKeyIdentifier = hash
EOF
openssl req -x509 -newkey rsa:2048 -nodes -sha256 -days 3650 -config "$work/cert.cnf" \
  -keyout "$work/key.pem" -out "$work/cert.pem" >/dev/null 2>&1 || stop "openssl could not make the certificate."

password=$(openssl rand -hex 24)
printf '%s' "$password" >"$out.password"
# Sealed the old way (3DES and SHA-1): the only PKCS#12 that macOS's own "security import" reads.
SEAL="$password" openssl pkcs12 -export -inkey "$work/key.pem" -in "$work/cert.pem" -name "$name" -out "$out" \
  -passout env:SEAL -keypbe PBE-SHA1-3DES -certpbe PBE-SHA1-3DES -macalg sha1 || stop "openssl could not write $out."

sha1=$(openssl x509 -in "$work/cert.pem" -noout -fingerprint -sha1 | sed 's/^.*=//' | tr -d ':')
echo "Certificate \"$name\", valid for ten years."
echo "SHA-1: $sha1"
echo "Wrote $out and $out.password. Keep both; neither belongs in the repository."
echo "To sign releases with it (neither command shows the certificate or its password):"
echo "  openssl base64 -A -in '$out' | gh secret set MAC_SIGN_P12 --repo $repo"
echo "  gh secret set MAC_SIGN_PASSWORD --repo $repo < '$out.password'"
