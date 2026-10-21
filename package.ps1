# Build Craftify and put what the computers it runs on need into dist\Craftify:
# the window (Craftify.exe), the command line (craftify-cli.exe) and cadmpeg.exe.
# Those computers need only SolidCraft installed (and Fusion, for Fusion designs).
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

cargo build --release --bins
if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

# The cadmpeg the converter would use: the patched build first, then an installed one.
$cadmpeg = @(
    (Join-Path $PSScriptRoot "packaging\cadmpeg\bin\cadmpeg.exe"),
    (Get-Command cadmpeg -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source),
    (Join-Path $env:USERPROFILE ".cargo\bin\cadmpeg.exe")
) | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
if (-not $cadmpeg) { throw "cadmpeg not found: cargo install cadmpeg --version 0.6.2 --locked" }

$dist = Join-Path $PSScriptRoot "dist\Craftify"
Remove-Item -Recurse -Force $dist -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $dist | Out-Null
Copy-Item target\release\craftify.exe (Join-Path $dist "Craftify.exe")
Copy-Item target\release\craftify-cli.exe $dist
Copy-Item $cadmpeg $dist
Copy-Item NOTICE.md, assets\fonts\OFL-IBMPlex.txt $dist
Get-ChildItem $dist | Select-Object Name, @{ n = "MB"; e = { [math]::Round($_.Length / 1MB, 1) } }
