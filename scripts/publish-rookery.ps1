<#
.SYNOPSIS
    Builds the tagged commit as a release and publishes it on the Rookery
    Isles website, where every installed viewer (0.10.4 and later) finds it
    within minutes and offers it in world: one click installs it and
    restarts the viewer.

.DESCRIPTION
    The same build, checks and files as the Release workflow's Windows job
    (.github/workflows/release.yml): the viewer built with the tag's version
    and the OpenXR loader built in, its version checked, packaged, zipped,
    its SHA-256 beside it, and the update policy. They are copied into the
    website's viewer\files\<tag> folder (rookeryisles.com
    lib/viewer_feed.php), first under a name the feed never lists, then
    renamed, so a viewer never sees half a release. The website's release
    list (/viewer/releases) is then read back to check the release is in it.

    HEAD must be the tagged commit, with nothing uncommitted. GitHub is not
    needed: the viewer reads the website first, GitHub only when the website
    cannot answer. Pushing the tag to GitHub stays a separate step.

.EXAMPLE
    ./scripts/publish-rookery.ps1 -Tag v0.10.4 -Notes C:\path\release-notes-v0.10.4.md
#>
param(
    [Parameter(Mandatory)][string]$Tag,
    [Parameter(Mandatory)][string]$Notes,
    [string]$Site = 'E:\rookeryisles_com\viewer\files'
)
$ErrorActionPreference = 'Stop'

function Invoke-Native([string]$What, [scriptblock]$Command) {
    & $Command
    if ($LASTEXITCODE -ne 0) { throw "$What failed (exit $LASTEXITCODE)" }
}

if ($Tag -notmatch '^v(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(-[0-9A-Za-z]+(\.[0-9A-Za-z]+)*)?$') {
    throw "Not a release tag: $Tag"
}
if (-not (Test-Path -LiteralPath $Notes -PathType Leaf)) { throw "No release notes at $Notes" }
if (-not (Test-Path -LiteralPath $Site -PathType Container)) { throw "No website release folder at $Site" }

$repo = (git -C $PSScriptRoot rev-parse --show-toplevel).Trim()
Push-Location $repo
try {
    # the commit built is the tag's, and nothing else goes in
    $exact = (git describe --tags --exact-match HEAD 2>$null)
    if ($LASTEXITCODE -ne 0 -or $exact.Trim() -ne $Tag) { throw "HEAD is not tagged $Tag (git tag $Tag first)" }
    $dirty = git status --porcelain --untracked-files=no
    if ($dirty) { throw "Uncommitted changes: commit them before publishing.`n$dirty" }

    $target = Join-Path $Site $Tag
    if (Test-Path -LiteralPath $target) { throw "$Tag is already published at $target" }

    Write-Host "Building $Tag..." -ForegroundColor Cyan
    $previous = $env:AURORA_VERSION_TAG
    try {
        $env:AURORA_VERSION_TAG = $Tag
        Invoke-Native 'cargo build' { cargo build --release --locked -p aurora-viewer --features xr-static-loader }
    } finally {
        $env:AURORA_VERSION_TAG = $previous
    }
    $exe = Join-Path $repo 'target\release\aurora-viewer.exe'
    & (Join-Path $repo 'scripts\verify-version.ps1') -Exe $exe -Tag $Tag

    # package, zip, checksum, policy: the Release workflow's Package step
    $work = Join-Path $repo "target\publish\$Tag"
    if (Test-Path -LiteralPath $work) { Remove-Item -LiteralPath $work -Recurse -Force }
    New-Item -ItemType Directory -Force -Path $work | Out-Null
    $name = "Rookery-Isles-Viewer-$Tag-windows-x64"
    $dir = Join-Path $work $name
    & (Join-Path $repo 'scripts\package.ps1') -Exe $exe -Out $dir
    $zip = Join-Path $work "$name.zip"
    Compress-Archive -Path $dir -DestinationPath $zip
    $hash = (Get-FileHash -LiteralPath $zip -Algorithm SHA256).Hash.ToLowerInvariant()
    $utf8 = New-Object System.Text.UTF8Encoding $false
    [IO.File]::WriteAllText("$zip.sha256", "$hash  $name.zip`n", $utf8)
    & (Join-Path $repo 'scripts\write-update-policy.ps1') -Tag $Tag -Out (Join-Path $work 'aurora-update-policy.json')

    # into the website: a folder the feed does not list, then its real name
    $partial = Join-Path $Site "$Tag.partial"
    if (Test-Path -LiteralPath $partial) { Remove-Item -LiteralPath $partial -Recurse -Force }
    New-Item -ItemType Directory -Path $partial | Out-Null
    Copy-Item -LiteralPath $zip, "$zip.sha256", (Join-Path $work 'aurora-update-policy.json') -Destination $partial
    [IO.File]::WriteAllText((Join-Path $partial 'notes.md'), [IO.File]::ReadAllText((Resolve-Path $Notes)), $utf8)
    $copied = (Get-FileHash -LiteralPath (Join-Path $partial "$name.zip") -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($copied -ne $hash) { throw "The copy on the website does not match the build ($copied, $hash)" }
    Rename-Item -LiteralPath $partial -NewName $Tag

    # read back what the viewers will read
    $list = Invoke-RestMethod -Uri 'http://127.0.0.1/viewer/releases' -Headers @{ Host = 'rookeryisles.com' }
    $listed = @($list | Where-Object { $_.tag_name -eq $Tag })
    if ($listed.Count -ne 1) { throw "$Tag is not in the website's release list" }
    $asset = @($listed[0].assets | Where-Object { $_.name -eq "$name.zip" })
    if ($asset.Count -ne 1 -or [int64]$asset[0].size -ne (Get-Item -LiteralPath $zip).Length) {
        throw "The website lists $Tag without its zip, or with the wrong size"
    }
    Write-Host "Published $Tag on rookeryisles.com: $([math]::Round((Get-Item -LiteralPath $zip).Length / 1MB)) MB, SHA-256 $hash" -ForegroundColor Green
    Write-Host 'Viewers from 0.10.4 on offer it within their next check (10 minutes by default).'
} finally {
    Pop-Location
}
