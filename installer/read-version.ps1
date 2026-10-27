# Prints the workspace version from the root Cargo.toml's
# [workspace.package] table. Dependency versions live in inline tables and
# are deliberately ignored. Used by prebuild.bat; safe to run standalone.
#
# Exits non-zero (with a message on stderr) when the table or key is missing,
# so prebuild.bat fails instead of building an installer with a bogus version.

$cargoToml = Join-Path $PSScriptRoot '..\Cargo.toml'
if (-not (Test-Path -LiteralPath $cargoToml -PathType Leaf)) {
    Write-Error "read-version: Cargo.toml not found at $cargoToml"
    exit 1
}

$inWorkspacePackage = $false
$version = $null

foreach ($line in Get-Content -LiteralPath $cargoToml) {
    if ($line -match '^\s*\[(?<section>[^\]]+)\]\s*$') {
        $inWorkspacePackage = ($Matches['section'] -eq 'workspace.package')
        continue
    }
    if ($inWorkspacePackage -and $line -match '^\s*version\s*=\s*"(?<version>[^"]+)"\s*(?:#.*)?$') {
        $version = $Matches['version']
        break
    }
}

if (-not $version) {
    Write-Error "read-version: no version key in the [workspace.package] table of $cargoToml"
    exit 1
}

if ($version -notmatch '^\d+\.\d+\.\d+$') {
    Write-Error "read-version: version '$version' is not numeric x.y.z (Inno Setup's AppVersion requires numeric)"
    exit 1
}

Write-Output $version
