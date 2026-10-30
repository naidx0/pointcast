# Install or update Pointcast on Windows 10 or 11. No admin rights, no prompts.
#
#   irm https://raw.githubusercontent.com/naidx0/pointcast/main/install.ps1 | iex
#
# To install a build you already have: $env:POINTCAST_SETUP = "C:\path\Pointcast_x64-setup.exe"
# Run as a script block so a failure never closes the PowerShell window it was pasted into.
& {
    $ErrorActionPreference = 'Stop'
    $ProgressPreference = 'SilentlyContinue'   # Windows PowerShell downloads 10x slower with the bar on
    # Add TLS 1.2 for GitHub without taking anything away from the rest of this session.
    [Net.ServicePointManager]::SecurityProtocol = [Net.ServicePointManager]::SecurityProtocol -bor [Net.SecurityProtocolType]::Tls12
    # Sign in to a proxy that asks for it as the current user; otherwise GitHub answers 407.
    if ([Net.WebRequest]::DefaultWebProxy) {
        [Net.WebRequest]::DefaultWebProxy.Credentials = [Net.CredentialCache]::DefaultNetworkCredentials
    }
    # Started from an elevated PowerShell, Pointcast would run as admin too: Open at login would
    # land in every account's startup and pasting into normal apps would fail.
    $admin = ([Security.Principal.WindowsPrincipal][Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole(
        [Security.Principal.WindowsBuiltInRole]::Administrator)
    $repo = 'naidx0/pointcast'
    $releases = "https://github.com/$repo/releases/latest"

    function Say([string]$m) { Write-Host "  $m" }
    function Stop-Install([string]$m) {
        Write-Host ""
        Write-Host "  Pointcast was not installed: $m" -ForegroundColor Red
        Write-Host ""
        # Closed for the update and the update failed: open the copy that is still there.
        if ($stopped) {
            $old = Find-Pointcast
            if ($old) { $null = Open-Pointcast $old }
        }
    }
    # Explorer starts it as the signed-in user, not as admin. False when Windows would not start it.
    function Open-Pointcast([string]$exe) {
        try {
            if ($admin) { Start-Process -FilePath explorer.exe -ArgumentList ('"' + $exe + '"') }
            else { Start-Process -FilePath $exe }
            return $true
        } catch {
            return $false
        }
    }
    function Find-Pointcast {
        foreach ($key in 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Pointcast',
                         'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\dev.pointcast.app') {
            $loc = (Get-ItemProperty -Path $key -ErrorAction SilentlyContinue).InstallLocation
            if ($loc) {
                $exe = Join-Path $loc.Trim('"') 'pointcast.exe'
                if (Test-Path $exe) { return $exe }
            }
        }
        $guess = Join-Path $env:LOCALAPPDATA 'Pointcast\pointcast.exe'
        if (Test-Path $guess) { return $guess }
        return $null
    }

    Write-Host ""
    Write-Host "  Pointcast" -ForegroundColor Cyan
    if (-not [Environment]::Is64BitOperatingSystem) {
        return Stop-Install "Pointcast needs 64-bit Windows."
    }
    $had = [bool](Find-Pointcast)

    $setup = $env:POINTCAST_SETUP
    if ($setup) {
        if (-not (Test-Path $setup)) { return Stop-Install "no file at $setup." }
    } else {
        Say "Finding the latest version..."
        try {
            $api = if ($env:POINTCAST_API) { $env:POINTCAST_API } else { "https://api.github.com/repos/$repo/releases/latest" }
            $rel = Invoke-RestMethod -UseBasicParsing -Headers @{ 'User-Agent' = 'pointcast-installer' } -Uri $api
        } catch {
            $code = 0
            try { $code = [int]$_.Exception.Response.StatusCode } catch { }
            if ($code -eq 404) { return Stop-Install "there is no published release yet. See https://github.com/$repo/releases." }
            if ($code -ne 403 -and $code -ne 429) {
                return Stop-Install "can't reach GitHub. Check your internet connection and run this again."
            }
            $rel = $null
        }
        if (-not $rel) {
            # Shared networks (offices, schools, VPNs) can use up GitHub's request allowance; the release's fixed
            # download link is not counted against it. The listed SHA-256 needs that allowance: HTTPS only here.
            Say "GitHub is busy on this network, so Pointcast downloads the latest version directly."
            $name = 'Pointcast-Windows-setup.exe'
            $url = "https://github.com/$repo/releases/latest/download/$name"
            $want = ''
        } else {
        $asset = $rel.assets | Where-Object { $_.name -like '*x64-setup.exe' } | Select-Object -First 1
        if (-not $asset) { return Stop-Install "the latest release ($($rel.tag_name)) has no Windows download yet. See $releases." }
        # GitHub lists a SHA-256 for every upload; without one the download cannot be checked.
        $want = if ("$($asset.digest)" -match '^sha256:([0-9a-fA-F]{64})$') { $Matches[1] } else { '' }
        if (-not $want) { return Stop-Install "GitHub lists no SHA-256 for $($asset.name), so the download cannot be checked. See $releases." }
        $name = $asset.name
        $url = $asset.browser_download_url
        }
        $setup = Join-Path $env:TEMP $name
        Say "Downloading $name..."
        try {
            Invoke-WebRequest -UseBasicParsing -Uri $url -OutFile $setup
        } catch {
            return Stop-Install "the download did not finish. Check your connection and run this again."
        }
        if ($want -and (Get-FileHash -Algorithm SHA256 -Path $setup).Hash -ne $want) {
            Remove-Item $setup -ErrorAction SilentlyContinue
            return Stop-Install "the download does not match the SHA-256 GitHub lists for it, so it was deleted. Run this again, or download the setup from $releases."
        }
    }
    Unblock-File -Path $setup -ErrorAction SilentlyContinue

    # An update replaces the running app; Pointcast keeps what you were recording for its next start.
    # Only this Windows session's copy: other signed-in users keep theirs.
    $session = (Get-Process -Id $PID).SessionId
    $running = @(Get-Process -Name pointcast -ErrorAction SilentlyContinue | Where-Object SessionId -eq $session)
    $stopped = $running.Count -gt 0
    $running | Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 600

    Say $(if ($had) { "Updating..." } else { "Installing..." })
    # /UPDATE keeps an update from removing the old copy first and from putting the shortcuts back.
    $switches = if ($had) { '/S', '/UPDATE' } else { '/S' }
    try {
        $p = Start-Process -FilePath $setup -ArgumentList $switches -PassThru
        $null = $p.Handle   # held now, so ExitCode is still there after it exits
    } catch {
        return Stop-Install "Windows blocked the installer. You can run it yourself: $setup"
    }
    # Not -Wait: in Windows PowerShell that also waits for anything the installer left running.
    $p.WaitForExit()
    if ($p.ExitCode -ne 0) {
        return Stop-Install "the installer stopped with code $($p.ExitCode). Close Pointcast if it is open and run this again. The installer is at $setup."
    }

    $exe = Find-Pointcast
    if (-not $exe) { return Stop-Install "the installer finished but pointcast.exe is not where it should be." }
    if (-not $env:POINTCAST_SETUP) { Remove-Item $setup -ErrorAction SilentlyContinue }

    if (-not (Open-Pointcast $exe)) {
        Say "Installed, but Windows did not start it. Open Pointcast from the Start menu."
    } elseif ($had) {
        Say "Updated. Pointcast is running in the system tray (bottom right, under the ^ arrow)."
    } else {
        Say "Installed. Pointcast is in the system tray (bottom right, under the ^ arrow)."
        Say "The setup assistant opens now. To uninstall: Settings > Apps > Pointcast."
    }
    Write-Host ""
}
