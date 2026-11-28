<#
CI-only software OpenGL for the real Windows capture window. The normal Glow
backend is unchanged. Nothing is installed into Windows or included in dist.
Source: mmozeiko/build-mesa@73f7cfb6d2ec725d1c036c33eba62f420f87818e
Mesa 26.3.1, LLVM 23.1.5. See docs/native-captures.md for sources and terms.
#>
param(
    [ValidateSet('x86_64-pc-windows-msvc', 'aarch64-pc-windows-msvc')]
    [string]$Target,
    [switch]$Cleanup
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

if ($env:GITHUB_ACTIONS -ne 'true') {
    throw 'This driver preparation is restricted to the ephemeral CI capture job.'
}

if ($Cleanup) {
    $InstalledLibrary = $env:ARCFORGE_CAPTURE_GL_LIBRARY_PATH
    if ($InstalledLibrary -and (Test-Path -LiteralPath $InstalledLibrary)) {
        if ([IO.Path]::GetFileName($InstalledLibrary) -ne 'opengl32.dll') {
            throw 'Refusing to remove an unexpected capture library name.'
        }
        $CurrentHash = (Get-FileHash -LiteralPath $InstalledLibrary -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($CurrentHash -ne $env:ARCFORGE_CAPTURE_GL_LIBRARY_SHA256) {
            throw 'Capture library changed; refusing to remove an unrecognized file.'
        }
        Remove-Item -LiteralPath $InstalledLibrary
    }
    exit 0
}

if (-not $Target -or -not $env:RUNNER_TEMP -or -not $env:GITHUB_ENV) {
    throw 'Target, RUNNER_TEMP and GITHUB_ENV are required.'
}

switch ($Target) {
    'x86_64-pc-windows-msvc' {
        $MesaArchitecture = 'x64'
        $ExpectedMachine = 0x8664
        $ArchiveHash = '08145a3717e0c1be0d97b74b53070f14eb39cb2f7de22f84fe20d1a581299c12'
    }
    'aarch64-pc-windows-msvc' {
        $MesaArchitecture = 'arm64'
        $ExpectedMachine = 0xaa64
        $ArchiveHash = 'c74c960c0416f3fa7f55dd877c34bbb1408f097cdf20a5638cd10ac1c90ae3c5'
    }
}

$CaptureApplicationDirectory = Join-Path $env:GITHUB_WORKSPACE "target/$Target/debug"
if (-not (Test-Path -LiteralPath (Join-Path $CaptureApplicationDirectory 'arcforge.exe'))) {
    throw 'Build the native debug capture application before preparing its driver.'
}
$InstalledLibrary = Join-Path $CaptureApplicationDirectory 'opengl32.dll'
if (Test-Path -LiteralPath $InstalledLibrary) {
    throw 'Refusing to overwrite an existing application OpenGL library.'
}
$DriverDirectory = Join-Path $env:RUNNER_TEMP ('arcforge-mesa-' + [Guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $DriverDirectory | Out-Null
$Archive = Join-Path $DriverDirectory 'mesa.7z'
$Uri = "https://github.com/mmozeiko/build-mesa/releases/download/26.3.1/mesa-llvmpipe-$MesaArchitecture-26.3.1.7z"
Invoke-WebRequest -Uri $Uri -OutFile $Archive -TimeoutSec 120
if ((Get-FileHash -LiteralPath $Archive -Algorithm SHA256).Hash.ToLowerInvariant() -ne $ArchiveHash) {
    throw 'Mesa archive SHA-256 does not match the pinned release asset.'
}

$SevenZip = Join-Path $env:ProgramFiles '7-Zip/7z.exe'
if (-not (Test-Path -LiteralPath $SevenZip)) {
    $SevenZip = (Get-Command 7z.exe -ErrorAction Stop).Source
}
& $SevenZip e $Archive 'opengl32.dll' "-o$DriverDirectory" -y -bso0 -bsp0
if ($LASTEXITCODE -ne 0) { throw 'Could not extract the pinned Mesa OpenGL library.' }
$Library = Join-Path $DriverDirectory 'opengl32.dll'

# Check the actual PE machine before copying a DLL for a native target.
$LibraryStream = [IO.File]::OpenRead($Library)
try {
    $Header = [byte[]]::new(64)
    if ($LibraryStream.Length -gt 256MB -or $LibraryStream.Read($Header, 0, 64) -ne 64 -or
        $Header[0] -ne 0x4d -or $Header[1] -ne 0x5a) {
        throw 'Mesa library has an invalid or excessive PE image.'
    }
    $PeOffset = [BitConverter]::ToUInt32($Header, 60)
    if ($PeOffset -lt 64 -or $PeOffset -gt $LibraryStream.Length - 6) {
        throw 'Mesa library has an invalid PE offset.'
    }
    $LibraryStream.Position = $PeOffset
    $PeHeader = [byte[]]::new(6)
    if ($LibraryStream.Read($PeHeader, 0, 6) -ne 6 -or
        [BitConverter]::ToUInt32($PeHeader, 0) -ne 0x00004550 -or
        [BitConverter]::ToUInt16($PeHeader, 4) -ne $ExpectedMachine) {
        throw 'Mesa library does not match the requested native architecture.'
    }
} finally {
    $LibraryStream.Dispose()
}
Copy-Item -LiteralPath $Library -Destination $InstalledLibrary
$LibraryHash = (Get-FileHash -LiteralPath $InstalledLibrary -Algorithm SHA256).Hash.ToLowerInvariant()
$EnvironmentValues = [ordered]@{
    ARCFORGE_CAPTURE_GL_DRIVER = 'Mesa 26.3.1 llvmpipe; software OpenGL; CI-only WGL driver'
    ARCFORGE_CAPTURE_GL_SOURCE_COMMIT = '73f7cfb6d2ec725d1c036c33eba62f420f87818e'
    ARCFORGE_CAPTURE_GL_PACKAGE_SHA256 = $ArchiveHash
    ARCFORGE_CAPTURE_GL_LIBRARY_SHA256 = $LibraryHash
    ARCFORGE_CAPTURE_GL_LICENSES = 'Mesa component licenses (mostly MIT); LLVM Apache-2.0 WITH LLVM-exception'
    # Write the cleanup pointer last, after the expected library hash.
    ARCFORGE_CAPTURE_GL_LIBRARY_PATH = $InstalledLibrary
}
foreach ($Entry in $EnvironmentValues.GetEnumerator()) {
    if ($Entry.Value -match '[\r\n]') { throw 'Unexpected newline in capture driver metadata.' }
    [IO.File]::AppendAllText($env:GITHUB_ENV, "$($Entry.Key)=$($Entry.Value)`n", [Text.UTF8Encoding]::new($false))
}
Write-Host "Prepared pinned Mesa 26.3.1 llvmpipe for native $Target capture; archive SHA-256 $ArchiveHash"
