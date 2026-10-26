param([string]$BootPath, [switch]$CheckOnly)
$ErrorActionPreference = 'Stop'

function Get-UpdatePlan([string]$BootRoot, [string]$PayloadRoot) {
    $resolvedBoot = (Resolve-Path -LiteralPath $BootRoot).ProviderPath.TrimEnd('\') + '\'
    if (!(Test-Path -LiteralPath (Join-Path $resolvedBoot 'config.txt')) -or
        !(Test-Path -LiteralPath (Join-Path $resolvedBoot 'cmdline.txt'))) {
        throw 'Choose the Raspberry Pi boot partition containing config.txt and cmdline.txt.'
    }
    if (!(Test-Path -LiteralPath (Join-Path $resolvedBoot 'bcm2711-rpi-4-b.dtb'))) {
        throw 'This does not look like a Raspberry Pi 4 boot partition.'
    }
    $manifest = Get-Content -LiteralPath (Join-Path $PayloadRoot 'manifest.json') -Raw | ConvertFrom-Json
    $package = Join-Path $PayloadRoot 'package.deb'
    if ((Get-FileHash -LiteralPath $package -Algorithm SHA256).Hash.ToLowerInvariant() -ne $manifest.sha256) {
        throw 'The update package checksum is incorrect. Extract the update ZIP again.'
    }
    $cmdlinePath = Join-Path $resolvedBoot 'cmdline.txt'
    $originalBytes = [IO.File]::ReadAllBytes($cmdlinePath)
    $cmdline = [Text.Encoding]::UTF8.GetString($originalBytes).Trim().TrimStart([char]0xFEFF)
    if ($cmdline -match '[\r\n\x00]' -or $cmdline -notmatch '(^|\s)root=') {
        throw 'cmdline.txt is not a normal single-line Raspberry Pi boot command line.'
    }
    if ($cmdline -match '(^|\s)(systemd\.run(?:_success_action|_failure_action)?|init)=') {
        throw 'Another first-boot or maintenance command is already scheduled. Finish that boot first.'
    }
    $target = Join-Path $resolvedBoot 'openstreamer-update'
    if (Test-Path -LiteralPath $target) { throw 'An openstreamer-update folder already exists on this card. Keep its logs and remove that folder before preparing another update.' }
    [pscustomobject]@{ Root=$resolvedBoot; Target=$target; Cmdline=$cmdlinePath; Original=$originalBytes; Text=$cmdline; Manifest=$manifest }
}

function Prepare-Update($Plan, [string]$PayloadRoot) {
    $expected = Join-Path $Plan.Root 'openstreamer-update'
    if ([IO.Path]::GetFullPath($Plan.Target) -ne [IO.Path]::GetFullPath($expected)) { throw 'Invalid update target.' }
    New-Item -ItemType Directory -Path $Plan.Target | Out-Null
    foreach ($name in @('package.deb','package.deb.sig','manifest.json','run.sh')) {
        Copy-Item -LiteralPath (Join-Path $PayloadRoot $name) -Destination (Join-Path $Plan.Target $name)
    }
    [IO.File]::WriteAllBytes((Join-Path $Plan.Target 'cmdline.before'), $Plan.Original)
    $newLine = $Plan.Text + ' systemd.run="/bin/bash /boot/firmware/openstreamer-update/run.sh" systemd.run_success_action=reboot systemd.run_failure_action=reboot'
    $temporary = Join-Path $Plan.Root 'cmdline.openstreamer-new'
    [IO.File]::WriteAllText($temporary, $newLine + "`n", [Text.UTF8Encoding]::new($false))
    Move-Item -LiteralPath $temporary -Destination $Plan.Cmdline -Force
}

# Dot-sourcing exposes the pure planning/preparation functions for fixture tests.
if ($MyInvocation.InvocationName -eq '.') { return }
Add-Type -AssemblyName System.Windows.Forms
try {
    if (!$BootPath) {
        $picker = New-Object System.Windows.Forms.FolderBrowserDialog
        $picker.Description = 'Select the Raspberry Pi microSD boot partition (config.txt and cmdline.txt).'
        $picker.ShowNewFolderButton = $false
        if ($picker.ShowDialog() -ne [Windows.Forms.DialogResult]::OK) { exit 0 }
        $BootPath = $picker.SelectedPath
    }
    $resolved = (Resolve-Path -LiteralPath $BootPath).ProviderPath
    $driveRoot = [IO.Path]::GetPathRoot($resolved)
    if ($resolved.TrimEnd('\') -ne $driveRoot.TrimEnd('\') -or
        $driveRoot.TrimEnd('\') -eq $env:SystemDrive) { throw 'Select the root of the microSD boot drive, not a directory or the Windows system drive.' }
    $drive = [IO.DriveInfo]::new($driveRoot)
    if ($drive.DriveFormat -notin @('FAT','FAT32')) { throw 'The Raspberry Pi boot partition must be FAT/FAT32.' }
    $payload = Join-Path $PSScriptRoot 'payload'
    $plan = Get-UpdatePlan $resolved $payload
    if ($CheckOnly) { Write-Output 'Boot partition and update package are valid.'; exit 0 }
    $answer = [Windows.Forms.MessageBox]::Show("Prepare Openstreamer $($plan.Manifest.version) on $driveRoot ?`nThe installed OS, settings and recordings will be kept.", 'Openstreamer update', 'YesNo', 'Question')
    if ($answer -ne [Windows.Forms.DialogResult]::Yes) { exit 0 }
    Prepare-Update $plan $payload
    [Windows.Forms.MessageBox]::Show('Update prepared. Safely eject the card, insert it into the powered-off Pi and turn it on. Wait for the automatic reboot. If it fails, result.txt and install.log are in openstreamer-update on this partition.', 'Openstreamer update') | Out-Null
} catch {
    [Windows.Forms.MessageBox]::Show($_.Exception.Message, 'Openstreamer update', 'OK', 'Error') | Out-Null
    exit 1
}
