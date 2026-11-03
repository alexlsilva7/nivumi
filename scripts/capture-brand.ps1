<#
.SYNOPSIS
Captures the real Nivumi renderer with synthetic data in a disposable debug profile.
Does not automate other apps or modify Windows display settings.
#>
[CmdletBinding()]
param()
$ErrorActionPreference = 'Stop'
$projectRoot = (Resolve-Path (Join-Path $PSScriptRoot '..')).Path
Push-Location -LiteralPath $projectRoot
$previousProfile = $env:NIVUMI_DATA_DIR
try {
    & cargo build --locked --offline
    if ($LASTEXITCODE -ne 0) { throw 'Debug build failed.' }
    $destination = Join-Path $projectRoot 'dist\verification'
    New-Item -ItemType Directory -Path $destination -Force | Out-Null
    $env:NIVUMI_DATA_DIR = Join-Path $projectRoot ('target\brand-visual-profiles\' + [guid]::NewGuid())
    $captureStarted = [DateTime]::UtcNow
    $proc = Start-Process -FilePath '.\target\debug\nivumi.exe' -ArgumentList @('--inspect', ('"--verify-brand=' + $destination + '"')) -WindowStyle Hidden -PassThru
    if (-not $proc.WaitForExit(30000)) { throw "Capture process $($proc.Id) did not finish; inspect it before retrying." }
    if ($proc.ExitCode -ne 0) { throw "Capture exited with $($proc.ExitCode)." }
    Add-Type -AssemblyName System.Drawing
    $bitmaps = @(Get-ChildItem -LiteralPath $destination -Filter '*.bmp')
    if ($bitmaps.Count -ne 27) { throw "Expected 27 frames, found $($bitmaps.Count)." }
    if (@($bitmaps | Where-Object LastWriteTimeUtc -LT $captureStarted).Count) { throw 'Some frames were not refreshed by this run.' }
    foreach ($file in $bitmaps) {
        $image = [System.Drawing.Image]::FromFile($file.FullName)
        try { $image.Save([System.IO.Path]::ChangeExtension($file.FullName, '.png'), [System.Drawing.Imaging.ImageFormat]::Png) }
        finally { $image.Dispose() }
    }
    $sheet = [System.Drawing.Bitmap]::new(1480, 1650)
    $graphics = [System.Drawing.Graphics]::FromImage($sheet)
    $font = [System.Drawing.Font]::new('Segoe UI', 12)
    $brush = [System.Drawing.SolidBrush]::new([System.Drawing.Color]::FromArgb(23,26,34))
    try {
        $graphics.Clear([System.Drawing.Color]::FromArgb(246,243,239))
        $frames = @(
            @('compact','Compacto sem mascote'),
            @('page-0-empty','Música sem reprodução'),
            @('page-0-media','Música · dados sintéticos'),
            @('page-1-lyrics','Letras e tradução · dados sintéticos'),
            @('page-2-empty','Pomodoro'), @('page-3-empty','Hidratação'),
            @('page-4-empty','Prateleira'), @('page-5-empty','Tarefas'),
            @('page-6-empty','Calendário')
        )
        for ($i=0; $i -lt $frames.Count; $i++) {
            $x = ($i % 2) * 740
            $y = [math]::Floor($i / 2) * 330
            $graphics.DrawString($frames[$i][1], $font, $brush, [single]($x+20), [single]($y+8))
            $image = [System.Drawing.Image]::FromFile((Join-Path $destination ('100-' + $frames[$i][0] + '.png')))
            try { $graphics.DrawImage($image, [System.Drawing.Rectangle]::new($x, ($y+36), 740, 286), [System.Drawing.Rectangle]::new(0,0,740,286), [System.Drawing.GraphicsUnit]::Pixel) }
            finally { $image.Dispose() }
        }
        $sheet.Save((Join-Path $destination 'overview.png'), [System.Drawing.Imaging.ImageFormat]::Png)
    } finally { $brush.Dispose(); $font.Dispose(); $graphics.Dispose(); $sheet.Dispose() }
    Write-Output "27 GPU captures and overview: $destination"
} finally {
    $env:NIVUMI_DATA_DIR = $previousProfile
    Pop-Location
}
