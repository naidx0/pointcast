<#
.SYNOPSIS
  Build the Windows installer and upload it to the matching GitHub release.
.DESCRIPTION
  Run from anywhere in the repo:  scripts/release-windows.ps1 [-NoUpload]
  Builds Pointcast_<version>_x64-setup.exe, prints its SHA-256 and size, uploads it to the
  release tagged v<version> (a draft is fine) and checks GitHub's digest against the local one.
#>
param([switch]$NoUpload)

$ErrorActionPreference = 'Stop'

function Fail([string]$Message) {
    Write-Host "FAILED: $Message" -ForegroundColor Red
    exit 1
}
function Step([string]$Message) {
    Write-Host ""
    Write-Host "==> $Message"
}
function Need([string]$Name, [string]$Hint) {
    if (-not (Get-Command $Name -ErrorAction SilentlyContinue)) { Fail "$Name is missing. $Hint" }
}

$root = Split-Path -Parent $PSScriptRoot
Set-Location $root
$conf = Join-Path $root 'src-tauri/tauri.conf.json'
if (-not (Test-Path $conf)) { Fail "src-tauri/tauri.conf.json not found; run this from the Pointcast repo." }

Step "Checking prerequisites"
Need 'node' 'Install Node.js 20 or newer from https://nodejs.org'
Need 'npm' 'Install Node.js 20 or newer from https://nodejs.org'
Need 'cargo' 'Install Rust from https://rustup.rs'
if (-not $NoUpload) {
    Need 'gh' 'Install the GitHub CLI from https://cli.github.com'
    & gh auth status *> $null
    if ($LASTEXITCODE -ne 0) { Fail "gh is not logged in. Run: gh auth login" }
}

$version = (Get-Content -Raw $conf | ConvertFrom-Json).version
if (-not $version) { Fail "could not read the version from src-tauri/tauri.conf.json." }
$tag = "v$version"
Write-Host "Version $version (release tag $tag)"

Step "Installing dependencies (npm ci)"
& npm ci
if ($LASTEXITCODE -ne 0) { Fail "npm ci did not complete." }

Step "Building the installer"
$log = Join-Path ([System.IO.Path]::GetTempPath()) 'pointcast-release-build.log'
$ErrorActionPreference = 'Continue'
& npm run tauri build -- --bundles nsis 2>&1 | ForEach-Object { if ($_ -is [System.Management.Automation.ErrorRecord]) { $_.Exception.Message } else { "$_" } } | Tee-Object -FilePath $log
$buildExit = $LASTEXITCODE
$ErrorActionPreference = 'Stop'
if ($buildExit -ne 0) {
    if (Select-String -Path $log -Pattern 'os error 4395' -Quiet) {
        Write-Host "This is a known stale-cache problem. Move %LOCALAPPDATA%/tauri/NSIS aside (rename it) and run this script again." -ForegroundColor Yellow
    }
    Fail "the Tauri build did not complete (exit code $buildExit). The full output is in $log"
}

$exe = Join-Path $root "src-tauri/target/release/bundle/nsis/Pointcast_${version}_x64-setup.exe"
if (-not (Test-Path $exe)) { Fail "the build finished but $exe was not produced." }

$sha = (Get-FileHash -Algorithm SHA256 -Path $exe).Hash.ToLower()
$size = (Get-Item $exe).Length
Write-Host ""
Write-Host "File:    $exe"
Write-Host "Size:    $size bytes"
Write-Host "SHA-256: $sha"

if ($NoUpload) {
    Write-Host ""
    Write-Host "Done (-NoUpload): build finished, nothing uploaded."
    exit 0
}

Step "Uploading to GitHub release $tag"
& gh release upload $tag $exe --clobber
if ($LASTEXITCODE -ne 0) { Fail "the upload to release $tag failed. Create the release (a draft is fine) with that tag first." }

Step "Confirming the uploaded file"
$repo = & gh repo view --json nameWithOwner -q .nameWithOwner
if ($LASTEXITCODE -ne 0 -or -not $repo) { Fail "could not read the repository name from gh." }
$name = Split-Path -Leaf $exe
$jq = ".[] | select(.tag_name == `"$tag`") | .assets[] | select(.name == `"$name`") | .digest"
$remote = & gh api "repos/$repo/releases" --paginate --jq $jq
if ($LASTEXITCODE -ne 0) { Fail "could not read the release assets from GitHub." }
$remote = ("$remote").Trim() -replace '^sha256:', ''
if (-not $remote) { Fail "GitHub reports no digest for $name on $tag." }
if ($remote -ne $sha) { Fail "GitHub's digest ($remote) does not match the local SHA-256 ($sha)." }

Write-Host "GitHub digest matches: $remote"
Write-Host ""
Write-Host "Done. $name is on release $tag. Publish the draft on GitHub when every platform is uploaded."
