<#
.SYNOPSIS
    Validates the release metadata that ci.yml and release.yml depend on.

.DESCRIPTION
    Checks, without building anything:

      * the workspace version (Cargo.toml [workspace.package]) equals the
        Tauri config version and the frontend package version;
      * an optional -Tag vX.Y.Z matches that version and is numeric;
      * CHANGELOG.md has a "## [<version>]" section;
      * the repository slug agrees across Cargo.toml, package.json,
        installer/selora-ide.iss, and docs/mkdocs.yml;
      * the About dialog and update check derive their URLs from those
        manifests instead of hard-coding a slug (a stale slug is how the
        update check returns 404);
      * the Tauri CSP still allows https://api.github.com for the update check.

    The release workflow used to inline the version checks in PowerShell and
    the slug lived in seven places. This script is the single gate: ci.yml
    runs it on every push, release.yml runs it with -Tag before building.

.PARAMETER Tag
    The release tag to validate (e.g. v0.1.0). When set, the tag must match
    the manifests and CHANGELOG.md must have a section for that version.

.PARAMETER RequireChangelog
    Require a CHANGELOG.md section for the current version even when no tag is
    supplied. ci.yml passes this on tag pushes only.

.PARAMETER RepoRoot
    Repository root. Defaults to the parent directory of this script.

.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File scripts\check-release-metadata.ps1

.EXAMPLE
    powershell -NoProfile -ExecutionPolicy Bypass -File scripts\check-release-metadata.ps1 -Tag v0.1.0

.NOTES
    PowerShell 5.1 compatible. No ternary operator, no PS7-only cmdlets.
    Exit code 0 means every check passed; 1 means at least one failed.
#>
[CmdletBinding()]
param(
    [string]$Tag = '',
    [switch]$RequireChangelog,
    [string]$RepoRoot = ''
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2.0

if ([string]::IsNullOrWhiteSpace($RepoRoot)) {
    $RepoRoot = Split-Path -Parent $PSScriptRoot
}
if (-not (Test-Path -LiteralPath $RepoRoot)) {
    Write-Host "FAIL repository root not found: $RepoRoot" -ForegroundColor Red
    exit 1
}
$RepoRoot = (Resolve-Path -LiteralPath $RepoRoot).Path

$failures = New-Object System.Collections.Generic.List[string]

function Add-Pass {
    param([string]$Message)
    Write-Host "[ok]   $Message" -ForegroundColor Green
}

function Add-Fail {
    param([string]$Message)
    $script:failures.Add($Message)
    Write-Host "[FAIL] $Message" -ForegroundColor Red
}

function Read-JsonFile {
    param([string]$RelativePath)

    $path = Join-Path $RepoRoot $RelativePath
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        Add-Fail "$RelativePath is missing"
        return $null
    }
    try {
        return (Get-Content -LiteralPath $path -Raw | ConvertFrom-Json)
    }
    catch {
        Add-Fail "$RelativePath is not valid JSON: $($_.Exception.Message)"
        return $null
    }
}

function Get-RepositoryUrl {
    param($Document)

    if ($null -eq $Document) {
        return ''
    }
    $property = $Document.PSObject.Properties.Match('repository')
    if ($property.Count -eq 0 -or $null -eq $Document.repository) {
        return ''
    }
    $value = $Document.repository
    if ($value -is [string]) {
        return $value
    }
    $urlProperty = $value.PSObject.Properties.Match('url')
    if ($urlProperty.Count -eq 0) {
        return ''
    }
    return [string]$value.url
}

function Normalize-RepositoryUrl {
    param([string]$Url)

    if ([string]::IsNullOrWhiteSpace($Url)) {
        return ''
    }
    $normalized = $Url.Trim()
    $normalized = $normalized -replace '\.git$', ''
    $normalized = $normalized.TrimEnd('/')
    return $normalized.ToLowerInvariant()
}

# --- Versions -----------------------------------------------------------

$cargoVersion = ''
try {
    $cargoVersion = (& (Join-Path $RepoRoot 'installer\read-version.ps1') | Out-String).Trim()
}
catch {
    Add-Fail "installer/read-version.ps1 failed: $($_.Exception.Message)"
}
if (-not $cargoVersion) {
    Add-Fail 'could not read [workspace.package].version from Cargo.toml'
}
else {
    Add-Pass "Cargo.toml [workspace.package].version is $cargoVersion"
}

$tauri = Read-JsonFile 'apps\selora-desktop\src-tauri\tauri.conf.json'
$frontend = Read-JsonFile 'apps\selora-desktop\frontend\package.json'

if ($null -ne $tauri) {
    if ($tauri.version -eq $cargoVersion -and $cargoVersion) {
        Add-Pass "tauri.conf.json version is $($tauri.version)"
    }
    else {
        Add-Fail "tauri.conf.json version is '$($tauri.version)' but Cargo.toml is '$cargoVersion'"
    }
}

if ($null -ne $frontend) {
    if ($frontend.version -eq $cargoVersion -and $cargoVersion) {
        Add-Pass "frontend/package.json version is $($frontend.version)"
    }
    else {
        Add-Fail "frontend/package.json version is '$($frontend.version)' but Cargo.toml is '$cargoVersion'"
    }
}

$releaseVersion = $cargoVersion
if (-not [string]::IsNullOrWhiteSpace($Tag)) {
    $tagVersion = $Tag.Trim() -replace '^v', ''
    if ($tagVersion -notmatch '^\d+\.\d+\.\d+$') {
        Add-Fail "tag '$Tag' is not numeric vX.Y.Z (Inno Setup's AppVersion must be numeric)"
    }
    elseif ($cargoVersion -and $tagVersion -ne $cargoVersion) {
        Add-Fail "tag '$Tag' is v$tagVersion but the manifests are $cargoVersion"
    }
    else {
        Add-Pass "tag $Tag matches the manifest version"
    }
    $releaseVersion = $tagVersion
}

if ($RequireChangelog -or -not [string]::IsNullOrWhiteSpace($Tag)) {
    $changelogPath = Join-Path $RepoRoot 'CHANGELOG.md'
    if (-not (Test-Path -LiteralPath $changelogPath -PathType Leaf)) {
        Add-Fail 'CHANGELOG.md is missing'
    }
    else {
        $changelog = Get-Content -LiteralPath $changelogPath -Raw
        $heading = '(?m)^##\s*\[' + [regex]::Escape($releaseVersion) + '\]'
        if ($changelog -match $heading) {
            Add-Pass "CHANGELOG.md has a '## [$releaseVersion]' section"
        }
        else {
            Add-Fail "CHANGELOG.md has no '## [$releaseVersion]' section; move the Unreleased entries into a versioned section before tagging"
        }
    }
}

# --- Repository slug ----------------------------------------------------

$cargoTomlPath = Join-Path $RepoRoot 'Cargo.toml'
$cargoRepo = ''
if (Test-Path -LiteralPath $cargoTomlPath -PathType Leaf) {
    $cargoText = Get-Content -LiteralPath $cargoTomlPath -Raw
    if ($cargoText -match '(?m)^\s*repository\s*=\s*"(?<url>[^"]+)"\s*$') {
        $cargoRepo = $Matches['url']
    }
}
if (-not $cargoRepo) {
    Add-Fail 'Cargo.toml has no [workspace.package] repository URL'
}

$frontendRepo = Get-RepositoryUrl -Document $frontend
if (-not $frontendRepo) {
    Add-Fail 'apps/selora-desktop/frontend/package.json has no repository URL'
}

$issPath = Join-Path $RepoRoot 'installer\selora-ide.iss'
$issRepo = ''
if (Test-Path -LiteralPath $issPath -PathType Leaf) {
    $issText = Get-Content -LiteralPath $issPath -Raw
    if ($issText -match '#define\s+RepoUrl\s+"(?<url>[^"]+)"') {
        $issRepo = $Matches['url']
    }
}
if (-not $issRepo) {
    Add-Fail 'installer/selora-ide.iss has no #define RepoUrl'
}

$mkdocsPath = Join-Path $RepoRoot 'docs\mkdocs.yml'
$mkdocsRepo = ''
if (Test-Path -LiteralPath $mkdocsPath -PathType Leaf) {
    $mkdocsText = Get-Content -LiteralPath $mkdocsPath -Raw
    if ($mkdocsText -match '(?m)^\s*repo_url:\s*(?<url>\S+)\s*$') {
        $mkdocsRepo = $Matches['url'].Trim('"').Trim("'")
    }
}
if (-not $mkdocsRepo) {
    Add-Fail 'docs/mkdocs.yml has no repo_url'
}

$normalized = @{}
foreach ($pair in @(
        @{ Name = 'Cargo.toml'; Url = $cargoRepo },
        @{ Name = 'frontend/package.json'; Url = $frontendRepo },
        @{ Name = 'installer/selora-ide.iss'; Url = $issRepo },
        @{ Name = 'docs/mkdocs.yml'; Url = $mkdocsRepo })) {
    if ([string]::IsNullOrWhiteSpace($pair.Url)) {
        continue
    }
    $normalized[$pair.Name] = Normalize-RepositoryUrl -Url $pair.Url
}

$canonical = ''
if ($normalized.ContainsKey('Cargo.toml')) {
    $canonical = $normalized['Cargo.toml']
}

if ($canonical) {
    if ($canonical -notmatch '^https://github\.com/[^/]+/[^/]+$') {
        Add-Fail "Cargo.toml repository '$cargoRepo' is not a canonical https://github.com/<owner>/<repo> URL"
    }
    if ($canonical -match 'example|your[-_]|changeme|placeholder|todo') {
        Add-Fail "Cargo.toml repository '$cargoRepo' still looks like a placeholder"
    }

    $mismatched = @()
    foreach ($name in $normalized.Keys) {
        if ($normalized[$name] -ne $canonical) {
            $mismatched += "$name ($($normalized[$name]))"
        }
    }
    if ($mismatched.Count -eq 0) {
        Add-Pass "repository slug agrees across Cargo.toml, package.json, selora-ide.iss, and mkdocs.yml ($canonical)"
    }
    else {
        Add-Fail "repository slug mismatch: expected $canonical; found $($mismatched -join ', ')"
    }
}

# The update check and the About dialog must derive from the manifests, not
# repeat a literal slug that can drift after the repository is renamed.
$updatesPath = Join-Path $RepoRoot 'crates\selora-ai\src\updates.rs'
if (Test-Path -LiteralPath $updatesPath -PathType Leaf) {
    $updatesText = Get-Content -LiteralPath $updatesPath -Raw
    if ($updatesText -match 'env!\("CARGO_PKG_REPOSITORY"\)') {
        Add-Pass 'crates/selora-ai update check derives its slug from CARGO_PKG_REPOSITORY'
    }
    else {
        Add-Fail 'crates/selora-ai/src/updates.rs hard-codes the repository; use env!("CARGO_PKG_REPOSITORY")'
    }
}

$aboutRustPath = Join-Path $RepoRoot 'crates\selora-ui\src\dialogs\about.rs'
if (Test-Path -LiteralPath $aboutRustPath -PathType Leaf) {
    $aboutRustText = Get-Content -LiteralPath $aboutRustPath -Raw
    if ($aboutRustText -match 'env!\("CARGO_PKG_REPOSITORY"\)') {
        Add-Pass 'crates/selora-ui About dialog derives its repo URL from CARGO_PKG_REPOSITORY'
    }
    else {
        Add-Fail 'crates/selora-ui/src/dialogs/about.rs hard-codes the repository URL'
    }
}

$aboutTsxPath = Join-Path $RepoRoot 'apps\selora-desktop\frontend\src\components\AboutDialog.tsx'
if (Test-Path -LiteralPath $aboutTsxPath -PathType Leaf) {
    $aboutTsxText = Get-Content -LiteralPath $aboutTsxPath -Raw
    if ($aboutTsxText -match '__REPOSITORY_URL__') {
        Add-Pass 'frontend About dialog derives its repo URL from package.json (Vite __REPOSITORY_URL__)'
    }
    else {
        Add-Fail 'apps/selora-desktop/frontend/src/components/AboutDialog.tsx hard-codes the repository URL'
    }
}

# Crate manifests must inherit the workspace repository. A literal URL in one
# crate is another place a rename can leave a stale slug.
$crateManifests = @(Get-ChildItem -LiteralPath (Join-Path $RepoRoot 'crates') -Filter 'Cargo.toml' -Recurse -ErrorAction SilentlyContinue)
$crateManifests += @(Get-ChildItem -LiteralPath (Join-Path $RepoRoot 'apps') -Filter 'Cargo.toml' -Recurse -ErrorAction SilentlyContinue)
$literalRepoManifests = @()
foreach ($manifest in $crateManifests) {
    $text = Get-Content -LiteralPath $manifest.FullName -Raw
    if ($text -match '(?m)^\s*repository\s*=\s*"') {
        $literalRepoManifests += $manifest.FullName.Substring($RepoRoot.Length).TrimStart('\')
    }
}
if ($crateManifests.Count -eq 0) {
    Add-Fail 'no crate Cargo.toml files found under crates/ or apps/'
}
elseif ($literalRepoManifests.Count -eq 0) {
    Add-Pass "crate manifests inherit the workspace repository ($($crateManifests.Count) checked)"
}
else {
    Add-Fail "crate manifests hard-code repository; use 'repository.workspace = true': $($literalRepoManifests -join ', ')"
}

# --- Update-check CSP ---------------------------------------------------

if ($null -ne $tauri) {
    $csp = [string]$tauri.app.security.csp
    if ($csp -and ($csp -match 'connect-src[^;]*https://api\.github\.com')) {
        Add-Pass 'tauri.conf.json CSP allows https://api.github.com for the update check'
    }
    else {
        Add-Fail 'tauri.conf.json CSP connect-src does not include https://api.github.com; the update check will be blocked'
    }
}

# --- Summary ------------------------------------------------------------

Write-Host ''
if ($failures.Count -gt 0) {
    Write-Host "Release metadata has $($failures.Count) problem(s):" -ForegroundColor Red
    foreach ($failure in $failures) {
        Write-Host "  - $failure" -ForegroundColor Red
    }
    exit 1
}

Write-Host "Release metadata is consistent (version $cargoVersion, repository $canonical)." -ForegroundColor Green
exit 0