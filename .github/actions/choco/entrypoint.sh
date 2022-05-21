#!/bin/bash

set -e

function choco {
  mono /opt/chocolatey/choco.exe "$@" --allow-unofficial --nocolor
}

function get_version {
  local version=${INPUT_VERSION:-$(git describe --tags)}
  version=(${version//[!0-9.-]/})
  local version_parts=(${version//-/ })
  version=${version_parts[0]}
  if [ ${#version_parts[@]} -gt 1 ]; then
    version=${version_parts}.${version_parts[1]}
  fi
  echo "$version"
}

## Determine the version to pack
VERSION=$(get_version)
echo "Packing version ${VERSION} of cargo-lambda"

mkdir -p tools
cp LICENSE tools/LICENSE.txt

cat > tools/VERIFICATION.txt<< EOF
Verification is intended to assist the Chocolatey moderators and community
in verifying that this package's contents are trustworthy.

Checksums: https://github.com/cargo-lambda/cargo-lambda/releases. 
Each release file has a checksum file associated with the same name, but with extension .sha256
EOF

cp cargo-lambda.exe tools/
choco pack cargo-lambda.nuspec --version ${VERSION} --out target
if [[ "$INPUT_PUSH" == "true" ]]; then
  choco push target/cargo-lambda.${VERSION}.nupkg --api-key ${INPUT_APIKEY} -s https://push.chocolatey.org/ --timeout 180
fi
