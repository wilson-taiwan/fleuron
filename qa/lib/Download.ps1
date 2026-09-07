# qa/lib/Download.ps1
# Fetches published releases for baseline and updater regression testing in Fleuron QA runner.

function Get-PreviousReleaseInstaller {
    param (
        [hashtable]$Evidence,
        [string]$PreviousLocalPath = "",
        [string]$Version = "1.2.0",
        [string]$DownloadDirectory = "$PSScriptRoot\..\download"
    )

    if ($PreviousLocalPath -and (Test-Path $PreviousLocalPath)) {
        $hash = (Get-FileHash -Path $PreviousLocalPath -Algorithm SHA256).Hash.ToLowerInvariant()
        Add-QATestCaseResult -Evidence $Evidence -Name "previous_installer_resolved" -Leg "updater" -Status "PASS" -Details "Using provided local previous installer: $PreviousLocalPath (SHA-256: $hash)"
        return $PreviousLocalPath
    }

    if (-not (Test-Path $DownloadDirectory)) {
        New-Item -ItemType Directory -Path $DownloadDirectory -Force | Out-Null
    }

    # Codemap-era baselines (0.x / 1.x) are deliberately NOT available online.
    # The v1.0.0 / v1.1.0 / v1.2.0 GitHub release records were removed on
    # 2026-08-28 by intent, so the release-asset URL returns 404. The git tags
    # survive, but GitHub serves only auto-generated SOURCE archives from a tag,
    # never the built installer -- there is no online source for a Codemap
    # installer and there is not meant to be one. Fail fast with an actionable
    # instruction instead of reporting a confusing download error.
    if ($Version -like "0.*" -or $Version -like "1.*") {
        $want = "Codemap_${Version}_x64-setup.exe"
        Add-QATestCaseResult -Evidence $Evidence -Name "previous_installer_download" -Leg "updater" -Status "FAIL" -ExitCode 1 -Details "Codemap $Version is not downloadable: its GitHub release record was intentionally removed. Re-run with -Previous pointing at a local copy of $want to exercise this leg." -Diagnostics "No online source for Codemap-era installers exists by design."
        return $null
    }

    $fileName = "Fleuron_${Version}_x64-setup.exe"

    $destPath = Join-Path $DownloadDirectory $fileName
    $downloadUrl = "https://github.com/wilsonhyeh/fleuron/releases/download/v${Version}/${fileName}"

    Write-Host "Downloading previous installer v${Version} from $downloadUrl..."
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    try {
        [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
        Invoke-WebRequest -Uri $downloadUrl -OutFile $destPath -UseBasicParsing
        $sw.Stop()

        if (-not (Test-Path $destPath) -or ((Get-Item $destPath).Length -eq 0)) {
            Add-QATestCaseResult -Evidence $Evidence -Name "previous_installer_download" -Leg "updater" -Status "FAIL" -ElapsedMs $sw.ElapsedMilliseconds -ExitCode 1 -Details "Downloaded previous installer was empty or missing at $destPath" -Diagnostics "URL: $downloadUrl"
            return $null
        }

        $hash = (Get-FileHash -Path $destPath -Algorithm SHA256).Hash.ToLowerInvariant()
        Add-QATestCaseResult -Evidence $Evidence -Name "previous_installer_download" -Leg "updater" -Status "PASS" -ElapsedMs $sw.ElapsedMilliseconds -Details "Downloaded v${Version} installer ($((Get-Item $destPath).Length) bytes, SHA-256: $hash)" -Diagnostics "URL: $downloadUrl"

        return $destPath
    } catch {
        $sw.Stop()
        Add-QATestCaseResult -Evidence $Evidence -Name "previous_installer_download" -Leg "updater" -Status "FAIL" -ElapsedMs $sw.ElapsedMilliseconds -ExitCode 1 -Details "Failed to download previous installer: $($_.Exception.Message)" -Diagnostics "URL: $downloadUrl"
        return $null
    }
}