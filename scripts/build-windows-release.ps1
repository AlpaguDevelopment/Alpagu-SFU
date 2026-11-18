# Build a Windows x64 release with notices and the test-client source.
param([string]$OutputDirectory = '')
$ErrorActionPreference = 'Stop'
$repo = Split-Path -Parent $PSScriptRoot
if (-not $IsWindows -and $env:OS -ne 'Windows_NT') { throw 'Build on Windows with the MSVC toolchain.' }
if (-not $OutputDirectory) { $OutputDirectory = Join-Path $repo 'artifacts' }
$OutputDirectory = [IO.Path]::GetFullPath($OutputDirectory)
$metadataJson = & cargo metadata --manifest-path (Join-Path $repo 'core/Cargo.toml') --locked --no-deps --format-version 1
if ($LASTEXITCODE -ne 0) { throw 'Cargo metadata failed' }
$version = (($metadataJson | ConvertFrom-Json).packages | Where-Object name -eq 'sfu-core').version
$name = "Alpagu-SFU-v$version-windows-x64"
$stage = Join-Path $OutputDirectory $name
if (Test-Path -LiteralPath $stage) { throw "Output already exists: $stage. Choose a new output directory." }
# Native dependency assertions can embed source paths. Build from a workspace-local vendor tree,
# rather than the developer's personal Cargo cache. The vendor tree is not distributed.
New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
$vendor = Join-Path $OutputDirectory 'vendor'
$vendorConfig = Join-Path $OutputDirectory 'vendor-config.toml'
$config = & cargo vendor --manifest-path (Join-Path $repo 'core/Cargo.toml') --locked $vendor
if ($LASTEXITCODE -ne 0) { throw 'Dependency vendoring failed' }
[IO.File]::WriteAllText($vendorConfig, ($config -join "`n"), [Text.UTF8Encoding]::new($false))
$oldFlags = $env:RUSTFLAGS
try {
    $env:RUSTFLAGS = "-C target-feature=+crt-static --remap-path-prefix=$repo=/src --remap-path-prefix=$env:USERPROFILE=/build/user"
    & cargo --config $vendorConfig build --manifest-path (Join-Path $repo 'core/Cargo.toml') --release --locked --target x86_64-pc-windows-msvc
    if ($LASTEXITCODE -ne 0) { throw 'Release build failed' }
} finally { $env:RUSTFLAGS = $oldFlags }
New-Item -ItemType Directory -Path $stage -Force | Out-Null
Copy-Item -LiteralPath (Join-Path $repo 'core/target/x86_64-pc-windows-msvc/release/sfu-core.exe') -Destination $stage
foreach ($file in @('README.md', 'LICENSE', 'CHANGELOG.md', 'CONTRIBUTING.md', 'SECURITY.md', 'THIRD_PARTY.md')) {
    Copy-Item -LiteralPath (Join-Path $repo $file) -Destination $stage
}
New-Item -ItemType Directory -Path (Join-Path $stage 'docs') | Out-Null
foreach ($file in @('protocol.md', 'roadmap.md', 'testing.md')) {
    Copy-Item -LiteralPath (Join-Path $repo "docs/$file") -Destination (Join-Path $stage 'docs')
}
$clientFiles = & git -C $repo ls-files -- test-client
if ($LASTEXITCODE -ne 0 -or -not $clientFiles) { throw 'No tracked test-client files found' }
foreach ($file in $clientFiles) {
    $target = Join-Path $stage $file
    New-Item -ItemType Directory -Path (Split-Path -Parent $target) -Force | Out-Null
    Copy-Item -LiteralPath (Join-Path $repo $file) -Destination $target
}
& python (Join-Path $PSScriptRoot 'collect-licenses.py') --output (Join-Path $stage 'third-party')
if ($LASTEXITCODE -ne 0) { throw 'License collection failed' }
& node (Join-Path $PSScriptRoot 'smoke-release.mjs') (Join-Path $stage 'sfu-core.exe')
if ($LASTEXITCODE -ne 0) { throw 'Release smoke test failed' }
$zip = Join-Path $OutputDirectory "$name.zip"
Compress-Archive -LiteralPath $stage -DestinationPath $zip
$hash = (Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLowerInvariant()
[IO.File]::WriteAllText((Join-Path $OutputDirectory 'SHA256SUMS.txt'), "$hash  $name.zip`n", [Text.UTF8Encoding]::new($false))
Write-Output "Created $zip"
