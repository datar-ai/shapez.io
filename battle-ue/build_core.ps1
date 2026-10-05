# Builds the Rust battle core and copies the DLL where the Unreal project
# expects it. Run from any folder:  powershell -ExecutionPolicy Bypass -File battle-ue\build_core.ps1
$ErrorActionPreference = "Stop"
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$core = Join-Path $here "..\battle-core"

Push-Location $core
try {
    cargo build --release --lib
    if ($LASTEXITCODE -ne 0) { throw "cargo build failed" }
} finally {
    Pop-Location
}

$release = Join-Path $core "target\release"
$lib = Join-Path $here "Source\ThirdParty\BattleCore\lib\Win64"
$bin = Join-Path $here "Binaries\Win64"
New-Item -ItemType Directory -Force -Path $lib, $bin | Out-Null
Copy-Item (Join-Path $release "battlecore.dll") $lib -Force
Copy-Item (Join-Path $release "battlecore.dll.lib") $lib -Force
Copy-Item (Join-Path $release "battlecore.dll") $bin -Force
Write-Host "battlecore.dll copied to $lib and $bin"
