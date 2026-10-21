# Build Craftify and put what the computers it runs on need into dist\Craftify: the window
# (Craftify.exe), the command line (craftify-cli.exe) and cadmpeg.exe. Those computers need only
# SolidCraft installed. Also writes the release downloads: dist\Craftify-<version>-setup.exe
# (the installer) and dist\Craftify-<version>-windows-x64.zip (the same files, portable).
$ErrorActionPreference = "Stop"
Set-Location $PSScriptRoot

cargo build --release --locked --bins
if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }

# The cadmpeg the converter would use: the patched build first, then an installed one.
$cadmpeg = @(
    (Join-Path $PSScriptRoot "packaging\cadmpeg\bin\cadmpeg.exe"),
    (Get-Command cadmpeg -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source),
    (Join-Path $env:USERPROFILE ".cargo\bin\cadmpeg.exe")
) | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
if (-not $cadmpeg) { throw "cadmpeg not found: powershell -ExecutionPolicy Bypass -File packaging\cadmpeg\build.ps1" }

$dist = Join-Path $PSScriptRoot "dist\Craftify"
Remove-Item -Recurse -Force $dist -ErrorAction SilentlyContinue
New-Item -ItemType Directory -Force $dist | Out-Null
Copy-Item target\release\craftify.exe (Join-Path $dist "Craftify.exe")
Copy-Item target\release\craftify-cli.exe $dist
Copy-Item $cadmpeg $dist
Copy-Item NOTICE.md, assets\fonts\OFL-IBMPlex.txt, packaging\README.txt $dist

$version = (Select-String -Path Cargo.toml -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
$zip = Join-Path $PSScriptRoot "dist\Craftify-$version-windows-x64.zip"
Remove-Item -Force $zip -ErrorAction SilentlyContinue
Compress-Archive -Path $dist -DestinationPath $zip

# The installer (Inno Setup 6): dist\Craftify-<version>-setup.exe.
$iscc = @(
    (Get-Command iscc -ErrorAction SilentlyContinue | Select-Object -ExpandProperty Source),
    (Join-Path ${env:ProgramFiles(x86)} "Inno Setup 6\ISCC.exe"),
    (Join-Path $env:LOCALAPPDATA "Programs\Inno Setup 6\ISCC.exe")
) | Where-Object { $_ -and (Test-Path $_) } | Select-Object -First 1
if (-not $iscc) { throw "Inno Setup 6 not found: winget install JRSoftware.InnoSetup" }
& $iscc /Q "/DAppVersion=$version" packaging\installer.iss
if ($LASTEXITCODE -ne 0) { throw "the installer did not build" }
$setup = Join-Path $PSScriptRoot "dist\Craftify-$version-setup.exe"

Get-ChildItem $dist, $zip, $setup | Select-Object Name, @{ n = "MB"; e = { [math]::Round($_.Length / 1MB, 1) } }
