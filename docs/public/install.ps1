#!/usr/bin/env pwsh

$ErrorActionPreference = 'Stop'

if ($v) {
  $Version = "v${v}"
}
if ($Args.Length -eq 1) {
  $Version = $Args.Get(0)
}

$CargoHome = $env:CARGO_HOME
$BinDir = if ($CargoHome) {
  "${CargoHome}\bin"
} else {
  "${Home}\.cargo\bin"
}

$CargoLambdaZip = "$BinDir\cargo-lambda.zip"
$CargoLambdaExe = "$BinDir\cargo-lambda.exe"
$Target = 'x86_64-pc-windows-msvc'

$Version = if (!$Version) {
  $LatestVersion = $curl.exe --ssl-revoke-best-effort -s "https://www.cargo-lambda.info/latest-version.json"
  $LatestVersion | ConvertFrom-Json | Select-Object -ExpandProperty latest
} else {
  $Version
}

$Name = "cargo-lambda-v${Version}.windows-x64.zip"
$DownloadUrl = "https://github.com/cargo-lambda/cargo-lambda/releases/download/v${Version}/${Name}"

if (!(Test-Path $BinDir)) {
  New-Item $BinDir -ItemType Directory | Out-Null
}

Write-Output "Downloading Cargo Lambda version ${Version}"
curl.exe --ssl-revoke-best-effort -Lo $CargoLambdaZip $DownloadUrl

Expand-Archive -Path $CargoLambdaZip -DestinationPath $BinDir

Remove-Item $CargoLambdaZip

Write-Output "Cargo Lambda was installed successfully to ${BinDir}"

Write-Output "Checking Zig installation"
$BinDir\cargo-lambda lambda system --install-zig

Write-Output "Installation complete! Run 'cargo lambda --help' to get started"
