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
            if ($old) { Start-Process -FilePath $old }
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
            $rel = Invoke-RestMethod -UseBasicParsing -Headers @{ 'User-Agent' = 'pointcast-installer' } `
                -Uri "https://api.github.com/repos/$repo/releases/latest"
        } catch {
            $code = 0
            try { $code = [int]$_.Exception.Response.StatusCode } catch { }
            if ($code -eq 404) { return Stop-Install "there is no published release yet. See https://github.com/$repo/releases." }
            if ($code -eq 403 -or $code -eq 429) {
                return Stop-Install "GitHub is limiting requests from your network. Try again in a few minutes, or download the setup from $releases."
            }
            return Stop-Install "can't reach GitHub. Check your internet connection and run this again."
        }
        $asset = $rel.assets | Where-Object { $_.name -like '*x64-setup.exe' } | Select-Object -First 1
        if (-not $asset) { return Stop-Install "the latest release ($($rel.tag_name)) has no Windows download yet. See $releases." }
        $setup = Join-Path $env:TEMP $asset.name
        Say "Downloading $($asset.name) ($([math]::Round($asset.size / 1MB)) MB)..."
        try {
            Invoke-WebRequest -UseBasicParsing -Uri $asset.browser_download_url -OutFile $setup
        } catch {
            return Stop-Install "the download did not finish. Check your connection and run this again."
        }
    }
    Unblock-File -Path $setup -ErrorAction SilentlyContinue

    # An update replaces the running app; Pointcast keeps what you were recording for its next start.
    $stopped = [bool](Get-Process -Name pointcast -ErrorAction SilentlyContinue)
    Get-Process -Name pointcast -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 600

    Say $(if ($had) { "Updating..." } else { "Installing..." })
    try {
        $p = Start-Process -FilePath $setup -ArgumentList '/S' -PassThru -Wait
    } catch {
        return Stop-Install "Windows blocked the installer. You can run it yourself: $setup"
    }
    if ($p.ExitCode -ne 0) {
        return Stop-Install "the installer stopped with code $($p.ExitCode). Close Pointcast if it is open and run this again. The installer is at $setup."
    }

    $exe = Find-Pointcast
    if (-not $exe) { return Stop-Install "the installer finished but pointcast.exe is not where it should be." }
    if (-not $env:POINTCAST_SETUP) { Remove-Item $setup -ErrorAction SilentlyContinue }

    Start-Process -FilePath $exe
    if ($had) {
        Say "Updated. Pointcast is running in the system tray (bottom right, under the ^ arrow)."
    } else {
        Say "Installed. Pointcast is in the system tray (bottom right, under the ^ arrow)."
        Say "The setup assistant opens now. To uninstall: Settings > Apps > Pointcast."
    }
    Write-Host ""
}
