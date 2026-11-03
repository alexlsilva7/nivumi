<#
.SYNOPSIS
Tests fresh install and legacy upgrade using a separate AppId, shortcuts and Run value.
Never installs over the user's Nivumi/Nusa Rebuild or runs either app.
#>
[CmdletBinding()]
param([string]$IsccPath = (Join-Path $PSScriptRoot '..\analysis\tools\inno-6.7.0\{app}\ISCC.exe'))
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$projectRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
$fixtureRoot = Join-Path $projectRoot 'dist\installer-verification'
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
$runName = 'NivumiVerification'
$fixtureId = 'A531C3D4-537A-4D64-9B9F-1557957B9A66'
$uninstallKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\{$fixtureId}_is1"
if (Test-Path -LiteralPath $uninstallKey) { throw 'A previous verification installation exists; clean up that test installation before retrying.' }
if (Get-ItemProperty -LiteralPath $runKey -Name $runName -ErrorAction SilentlyContinue) { throw 'Verification Run value already exists.' }
New-Item -ItemType Directory -Path $fixtureRoot -Force | Out-Null
$payload = Join-Path $projectRoot 'target\release\nivumi.exe'
$oldPayload = Join-Path $fixtureRoot 'nusa-rebuild.exe'
Copy-Item -LiteralPath $payload -Destination $oldPayload -Force
$dataRoot = Join-Path $fixtureRoot 'profile'
New-Item -ItemType Directory -Path $dataRoot -Force | Out-Null
$sentinel = Join-Path $dataRoot 'state.json'
Set-Content -LiteralPath $sentinel -Value '{"verification":"preserve existing profile"}' -Encoding utf8
$before = (Get-FileHash -LiteralPath $sentinel).Hash
$freshDir = Join-Path $fixtureRoot 'fresh-install'
$oldDir = Join-Path $fixtureRoot 'previous-install'
$newText = Get-Content -LiteralPath (Join-Path $projectRoot 'packaging\installer.iss') -Raw
$oldText = Get-Content -LiteralPath (Join-Path $PSScriptRoot 'fixtures\legacy-installer.iss') -Raw
function Compile-Fixture([string]$text, [string]$name, [string]$source, [string]$defaultDir) {
    $text = $text.Replace('B3E7D092-51F9-4D95-B91B-806126794091', $fixtureId)
    $text = $text.Replace('Nusa Rebuild', 'Nusa Rebuild Verification').Replace('Nivumi', 'Nivumi Verification')
    $text = $text.Replace('Nivumi Verification.Desktop', 'NivumiVerification.Desktop')
    $text = $text.Replace('Local\NusaRebuild.SingleInstance', 'Local\NivumiVerification.SingleInstance')
    $text = $text.Replace('ValueName: "NusaRebuild"', 'ValueName: "NivumiVerification"').Replace("'NusaRebuild'", "'NivumiVerification'")
    $text = $text.Replace('{localappdata}\NusaRebuild', $dataRoot)
    $lines = foreach ($line in ($text -split '\r?\n')) {
        if ($line.StartsWith('DefaultDirName=')) { 'DefaultDirName=' + $defaultDir }
        elseif ($line.StartsWith('OutputDir=')) { 'OutputDir=' + $fixtureRoot }
        elseif ($line.StartsWith('OutputBaseFilename=')) { 'OutputBaseFilename=' + $name }
        elseif ($line.StartsWith('SetupIconFile=')) { 'SetupIconFile=' + (Join-Path $projectRoot 'assets\brand\app.ico') }
        elseif ($line.StartsWith('Source: ')) { $line -replace '^Source: "[^"]+"', ('Source: "' + $source + '"') }
        else { $line }
    }
    $iss = Join-Path $fixtureRoot ($name + '.iss')
    $lines | Set-Content -LiteralPath $iss -Encoding utf8BOM
    & $IsccPath /Q '/DAppVersion=0.2.0' $iss | Write-Host
    if ($LASTEXITCODE -ne 0) { throw "Fixture compilation failed: $name" }
    return (Join-Path $fixtureRoot ($name + '.exe'))
}
function Run-Setup([string]$exe, [string[]]$extraArgs = @()) {
    $proc = Start-Process -FilePath $exe -ArgumentList (@('/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART','/SP-') + $extraArgs) -WindowStyle Hidden -Wait -PassThru
    if ($proc.ExitCode -ne 0) { throw "Setup/uninstall exit code $($proc.ExitCode): $exe" }
}
function Assert-Profile { if ((Get-FileHash -LiteralPath $sentinel).Hash -ne $before) { throw 'Existing profile changed.' } }
function Assert-Payload([string]$directory) {
    $installed = Join-Path $directory 'nivumi.exe'
    if ((Get-FileHash -LiteralPath $installed).Hash -ne (Get-FileHash -LiteralPath $payload).Hash) { throw 'Installed payload differs from release.' }
    if (Test-Path -LiteralPath (Join-Path $directory 'nusa-rebuild.exe')) { throw 'Obsolete executable was not removed.' }
    $programs = [Environment]::GetFolderPath('Programs')
    $shortcut = Join-Path $programs 'Nivumi Verification.lnk'
    if (-not (Test-Path -LiteralPath $shortcut)) { throw 'New Start menu shortcut missing.' }
    if (Test-Path -LiteralPath (Join-Path $programs 'Nusa Rebuild Verification.lnk')) { throw 'Legacy Start menu shortcut still exists.' }
    $shell = New-Object -ComObject WScript.Shell
    $link = $shell.CreateShortcut($shortcut)
    if ($link.TargetPath -ne $installed) { throw 'Start menu shortcut targets the wrong executable.' }
    $explorer = New-Object -ComObject Shell.Application
    $item = $explorer.Namespace($programs).ParseName('Nivumi Verification.lnk')
    if ($item.ExtendedProperty('System.AppUserModel.ID') -ne 'NivumiVerification.Desktop') { throw 'Shortcut AppUserModelID mismatch.' }
    Assert-Profile
}
$results = [System.Collections.Generic.List[string]]::new()
try {
    $oldSetup = Compile-Fixture $oldText 'LegacyVerificationSetup' $oldPayload $oldDir
    $newSetup = Compile-Fixture $newText 'NivumiVerificationSetup' $payload $freshDir
    Run-Setup $newSetup
    Assert-Payload $freshDir
    if (Get-ItemProperty -LiteralPath $runKey -Name $runName -ErrorAction SilentlyContinue) { throw 'Fresh install enabled startup.' }
    Run-Setup (Join-Path $freshDir 'unins000.exe')
    Assert-Profile
    $results.Add('Fresh install and silent uninstall preserve data and leave startup disabled.')
    foreach ($startup in @($true, $false)) {
        Run-Setup $oldSetup
        if (-not (Test-Path -LiteralPath (Join-Path $oldDir 'nusa-rebuild.exe'))) { throw 'Legacy fixture not installed.' }
        if ($startup) {
            if (-not (Test-Path -LiteralPath $runKey)) { New-Item -Path $runKey | Out-Null }
            New-ItemProperty -LiteralPath $runKey -Name $runName -Value ('"' + (Join-Path $oldDir 'nusa-rebuild.exe') + '"') -PropertyType String -Force | Out-Null
        }
        Run-Setup $newSetup
        Assert-Payload $oldDir
        $registration = Get-ItemProperty -LiteralPath $runKey -Name $runName -ErrorAction SilentlyContinue
        if ($startup) {
            if ($registration.$runName -ne ('"' + (Join-Path $oldDir 'nivumi.exe') + '"')) { throw 'Startup path not updated.' }
        } elseif ($registration) { throw 'Disabled startup was reenabled.' }
        Run-Setup (Join-Path $oldDir 'unins000.exe')
        Assert-Profile
        $results.Add("Legacy upgrade preserves install path and data; startup enabled=$startup.")
    }
    $results | ConvertTo-Json | Set-Content -LiteralPath (Join-Path $fixtureRoot 'results.json') -Encoding utf8
    $results
} finally {
    foreach ($dir in @($oldDir, $freshDir)) {
        $uninstaller = Join-Path $dir 'unins000.exe'
        if (Test-Path -LiteralPath $uninstaller) { Run-Setup $uninstaller }
    }
    Remove-ItemProperty -LiteralPath $runKey -Name $runName -ErrorAction SilentlyContinue
}
