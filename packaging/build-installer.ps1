# Builds the release binary and packs it into target/installer/Scratchpad-<version>-x64.msi.
#
#   powershell -File packaging/build-installer.ps1
#
# Needs the Rust toolchain and the .NET SDK. WiX is a repo-local dotnet tool (.config/dotnet-tools.json),
# restored into the user's NuGet cache; nothing is installed system-wide.
$ErrorActionPreference = 'Stop'
$env:DOTNET_CLI_TELEMETRY_OPTOUT = '1'
$env:DOTNET_NOLOGO = '1'

function Invoke-Native([scriptblock]$Command) {
    & $Command
    if ($LASTEXITCODE -ne 0) { throw "Command failed with exit code ${LASTEXITCODE}: $Command" }
}

$root = Split-Path -Parent $PSScriptRoot
Push-Location $root
try {
    Invoke-Native { cargo build --release -p scratchpad }

    $metadata = cargo metadata --no-deps --format-version 1 | ConvertFrom-Json
    $version = $metadata.packages | Where-Object name -eq 'scratchpad' | ForEach-Object version
    if (-not $version) { throw 'Could not read the scratchpad version from cargo metadata.' }
    $exe = Join-Path $metadata.target_directory 'release/scratchpad.exe'

    # The tool manifest names no package source, and a clean machine may have none configured.
    Invoke-Native { dotnet tool restore --add-source https://api.nuget.org/v3/index.json }

    $installerDir = Join-Path $metadata.target_directory 'installer'
    New-Item -ItemType Directory -Force $installerDir | Out-Null
    $msi = Join-Path $installerDir "Scratchpad-$version-x64.msi"
    Invoke-Native {
        dotnet tool run wix build -arch x64 -o $msi `
            -d "Version=$version" `
            -d "ExePath=$exe" `
            -d "IconPath=$root/assets/icons/scratchpad.ico" `
            packaging/scratchpad.wxs
    }
    # `wix build` does not run the ICE consistency checks; this does. ICE61 (same-version upgrades)
    # and ICE91 (fixed per-user folder) are intended, see ADR 0091.
    Invoke-Native { dotnet tool run wix msi validate -sice ICE61 -sice ICE91 $msi }
    Write-Host "Built $msi ($([math]::Round((Get-Item $msi).Length / 1MB, 2)) MB)"
}
finally {
    Pop-Location
}
