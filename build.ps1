param([switch]$Run)

$ErrorActionPreference = 'Stop'
Set-Location -LiteralPath $PSScriptRoot
$cargoCommand = Get-Command cargo -ErrorAction SilentlyContinue
$cargoExecutable = if ($cargoCommand) { $cargoCommand.Source } else { Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe' }
if (-not (Test-Path -LiteralPath $cargoExecutable)) {
    throw 'Rust is required. Install Rust, then run this script again.'
}

# Strip the builder's username and source directory from panic locations and
# dependency paths embedded in the executable. Never hard-code local paths.
$previousFlags = $env:CARGO_ENCODED_RUSTFLAGS
$releaseFlags = @()
if ($previousFlags) {
    $releaseFlags += $previousFlags -split [char]31
} elseif ($env:RUSTFLAGS) {
    $releaseFlags += $env:RUSTFLAGS -split '\s+' | Where-Object { $_ }
}
foreach ($sourceRoot in @($env:USERPROFILE, $env:CARGO_HOME, $env:RUSTUP_HOME)) {
    if ($sourceRoot) { $releaseFlags += "--remap-path-prefix=$sourceRoot=/build/home" }
}
$releaseFlags += "--remap-path-prefix=$PSScriptRoot=/build/klaso-desktop"
$env:CARGO_ENCODED_RUSTFLAGS = $releaseFlags -join [char]31
try {
    $cargoAction = if ($Run) { 'run' } else { 'build' }
    & $cargoExecutable $cargoAction --release --locked
    if ($LASTEXITCODE -ne 0) { throw "Cargo $cargoAction failed with exit code $LASTEXITCODE." }
} finally {
    $env:CARGO_ENCODED_RUSTFLAGS = $previousFlags
}
