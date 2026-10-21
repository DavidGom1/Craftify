# Builds the cadmpeg the converter uses: cadmpeg with only the readers it needs (f3d, sldprt,
# step) and the patches in this folder applied to its Fusion reader. The result lands in
# packaging\cadmpeg\bin\cadmpeg.exe, which the converter and the packaged app pick up.
#
# Needs Rust (rustup) and about 20 minutes the first time; later builds reuse .\target.
#
#   powershell -ExecutionPolicy Bypass -File packaging\cadmpeg\build.ps1

$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$cadmpeg = "0.6.2"      # the CLI crate
$codec = "0.6.3"        # the cadmpeg-codec-f3d version it resolves to
$env:PATH = "$env:USERPROFILE\.cargo\bin;$env:PATH"

# The f3d reader's source, from crates.io, with our patches applied.
$work = Join-Path $here "work"
$src = Join-Path $work "cadmpeg-codec-f3d-$codec"
if (Test-Path $src) { Remove-Item -Recurse -Force $src }
New-Item -ItemType Directory -Force $work | Out-Null
$crate = Join-Path $work "cadmpeg-codec-f3d-$codec.crate"
Invoke-WebRequest "https://static.crates.io/crates/cadmpeg-codec-f3d/$codec/download" -OutFile $crate
tar -xzf $crate -C $work
Get-ChildItem (Join-Path $here "*.patch") | Sort-Object Name | ForEach-Object {
    git -C $src apply --ignore-whitespace $_.FullName
    if ($LASTEXITCODE -ne 0) { throw "patch $($_.Name) does not apply" }
}

$patch = "patch.crates-io.cadmpeg-codec-f3d.path='" + ($src -replace "\\", "/") + "'"
cargo install cadmpeg --version $cadmpeg --no-default-features --features "f3d,sldprt,step" `
    --root (Join-Path $here "install") --target-dir (Join-Path $here "target") --force --config $patch
if ($LASTEXITCODE -ne 0) { throw "cargo install failed" }

New-Item -ItemType Directory -Force (Join-Path $here "bin") | Out-Null
Copy-Item -Force (Join-Path $here "install\bin\cadmpeg.exe") (Join-Path $here "bin\cadmpeg.exe")
Write-Host "cadmpeg built: $(Join-Path $here 'bin\cadmpeg.exe')"
