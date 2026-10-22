# Fetches the solidcraft-cli the converter uses, so the computers Craftify runs on need nothing
# else installed: the official SolidCraft release for Windows x64 (the portable zip), checked
# against the checksum below. solidcraft-cli.exe and SolidCraft's licence files land in
# packaging\solidcraft\bin, which the converter and the packaged app pick up.
#
# The version is fixed on purpose: SolidCraft's design format changes between releases, and a
# new one is taken by changing $version and $sha256 here (SHA256SUMS.txt of that release) and the
# version in NOTICE.md and README.md, and converting the test files again.
#
#   powershell -ExecutionPolicy Bypass -File packaging\solidcraft\fetch.ps1

$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$version = "0.19.0"
$sha256 = "e4eb659123be862a5642c7b7828e18f94a91ecbc4b147e6ee71319cf410a79ec"
$name = "solidcraft-$version-windows-x64-portable"

$bin = Join-Path $here "bin"
$cli = Join-Path $bin "solidcraft-cli.exe"
if ((Test-Path $cli) -and ((& $cli --version) -eq "solidcraft-cli $version")) {
    Write-Host "solidcraft-cli $version is already in $bin"
    exit 0
}

$work = Join-Path $here "work"
New-Item -ItemType Directory -Force $work | Out-Null
$zip = Join-Path $work "$name.zip"
if (-not (Test-Path $zip) -or (Get-FileHash $zip -Algorithm SHA256).Hash -ne $sha256) {
    $ProgressPreference = "SilentlyContinue"   # the progress bar makes the download many times slower
    Invoke-WebRequest "https://github.com/tangency-labs/solidcraft/releases/download/v$version/$name.zip" -OutFile $zip
}
$got = (Get-FileHash $zip -Algorithm SHA256).Hash
if ($got -ne $sha256) { throw "$name.zip: SHA-256 $got, expected $sha256" }

$unzipped = Join-Path $work $name
if (Test-Path $unzipped) { Remove-Item -Recurse -Force $unzipped }
Expand-Archive $zip -DestinationPath $work
if (Test-Path $bin) { Remove-Item -Recurse -Force $bin }
New-Item -ItemType Directory -Force $bin | Out-Null
foreach ($f in "solidcraft-cli.exe", "LICENSE-MIT", "LICENSE-APACHE", "NOTICE", "THIRD-PARTY-LICENSES.txt") {
    Copy-Item (Join-Path $unzipped $f) $bin
}
Remove-Item -Recurse -Force $unzipped
& $cli --version
