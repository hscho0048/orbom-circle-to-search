param([ValidateSet('all', 'x64', 'arm64')][string]$Architecture = 'all')
$ErrorActionPreference = 'Stop'
$originalPath = $env:PATH
$originalLib = $env:LIB
$originalLinker = $env:CARGO_TARGET_AARCH64_PC_WINDOWS_MSVC_LINKER
$originalPayload = $env:ORBOM_EXE

function Use-Arm64Toolchain {
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio\Installer\vswhere.exe'
    $installation = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.ARM64 -property installationPath
    if (-not $installation) { throw 'Visual Studio C++ Build Tools is required.' }
    $msvc = Get-ChildItem (Join-Path $installation 'VC\Tools\MSVC') -Directory | Sort-Object Name -Descending | Select-Object -First 1
    $sdk = Get-ChildItem (Join-Path ${env:ProgramFiles(x86)} 'Windows Kits\10\Lib') -Directory | Sort-Object Name -Descending | Select-Object -First 1
    $taskLib = Join-Path $msvc.FullName 'lib\arm64'
    if (-not (Test-Path (Join-Path $taskLib 'msvcrt.lib'))) {
        $localLib = Join-Path $PSScriptRoot "target\arm64-toolchain\crt\Contents\VC\Tools\MSVC\$($msvc.Name)\lib\arm64"
        if (-not (Test-Path (Join-Path $localLib 'msvcrt.lib'))) {
            throw 'Install Visual Studio C++ ARM64 build tools, then run again.'
        }
        $taskLib = $localLib
    }
    $env:CARGO_TARGET_AARCH64_PC_WINDOWS_MSVC_LINKER = Join-Path $msvc.FullName 'bin\Hostx64\x64\link.exe'
    $env:LIB = "$taskLib;$($sdk.FullName)\ucrt\arm64;$($sdk.FullName)\um\arm64"
    $sdkBin = Join-Path ${env:ProgramFiles(x86)} "Windows Kits\10\bin\$($sdk.Name)\x64"
    $env:PATH = "$sdkBin;$originalPath"
}

# Builds the app, then an installer that embeds it: dist\Orbom-Setup-<arch>.exe
function Build-Setup([string]$arch) {
    $targetArgs = @()
    $out = 'target\release'
    if ($arch -eq 'arm64') {
        Use-Arm64Toolchain
        $targetArgs = @('--target', 'aarch64-pc-windows-msvc')
        $out = 'target\aarch64-pc-windows-msvc\release'
    }
    cargo build --locked --release -p orbom @targetArgs
    if ($LASTEXITCODE -ne 0) { throw "App build failed ($arch)." }
    $env:ORBOM_EXE = (Resolve-Path "$out\orbom.exe").Path
    cargo build --locked --release -p orbom-setup @targetArgs
    if ($LASTEXITCODE -ne 0) { throw "Installer build failed ($arch)." }
    New-Item -ItemType Directory -Path 'dist' -Force | Out-Null
    $setup = "dist\Orbom-Setup-$arch.exe"
    Copy-Item -LiteralPath "$out\orbom-setup.exe" -Destination $setup -Force
    Write-Output $setup
}

Push-Location $PSScriptRoot
try {
    $architectures = if ($Architecture -eq 'all') { @('x64', 'arm64') } else { @($Architecture) }
    foreach ($arch in $architectures) {
        Build-Setup $arch
        $env:PATH = $originalPath
        $env:LIB = $originalLib
        $env:CARGO_TARGET_AARCH64_PC_WINDOWS_MSVC_LINKER = $originalLinker
    }
} finally {
    $env:PATH = $originalPath
    $env:LIB = $originalLib
    $env:CARGO_TARGET_AARCH64_PC_WINDOWS_MSVC_LINKER = $originalLinker
    $env:ORBOM_EXE = $originalPayload
    Pop-Location
}
