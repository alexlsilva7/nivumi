<#
.SYNOPSIS
    Compila e empacota o Nivumi para distribuição.

.DESCRIPTION
    Script oficial de build e distribuição. Executa verificações de qualidade,
    compila o binário em modo Release e empacota os artefatos portáteis (diretório,
    arquivo ZIP e manifesto SHA256SUMS.txt em dist/).
    Opcionalmente compila o instalador Inno Setup e/ou inicia o executável gerado.

.PARAMETER Installer
    Compila o instalador Inno Setup (dist/NivumiSetup.exe).

.PARAMETER IsccPath
    Caminho explícito para o executável do compilador Inno Setup (ISCC.exe).

.PARAMETER SkipChecks
    Ignora as verificações prévias de código (fmt, test, clippy).

.PARAMETER Run
    Executa o aplicativo gerado após a conclusão do build.

.PARAMETER KillExisting
    Fecha instâncias anteriores do nivumi.exe antes de compilar/copiar.

.PARAMETER NoZip
    Gera o executável e a pasta portátil sem compactar em arquivo ZIP.

.PARAMETER Clean
    Limpa o diretório dist/ antes de iniciar o processo de build.
#>
[CmdletBinding()]
param(
    [switch]$Installer,
    [string]$IsccPath = $(if ($env:NIVUMI_ISCC_PATH) { $env:NIVUMI_ISCC_PATH } else { $env:NUSA_ISCC_PATH }),
    [switch]$SkipChecks,
    [switch]$Run,
    [switch]$KillExisting,
    [switch]$NoZip,
    [switch]$Clean
)
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
$projectRoot = Split-Path -Parent $PSScriptRoot
Push-Location -LiteralPath $projectRoot
try {
    # 0. Tratar instâncias anteriores em execução
    $existing = Get-Process -Name 'nivumi','nusa-rebuild' -ErrorAction SilentlyContinue
    if ($existing) {
        if ($KillExisting) {
            Write-Host "Encerando $(@($existing).Count) instância(s) anterior(es) de nivumi (PID $(@($existing.Id) -join ', '))..." -ForegroundColor Yellow
            $existing | Stop-Process -Force -ErrorAction SilentlyContinue
            Start-Sleep -Milliseconds 400
        } else {
            Write-Host "Aviso: Já existe uma instância de nivumi em execução (PID $(@($existing.Id) -join ', '))." -ForegroundColor Yellow
            Write-Host "Dica: Use -KillExisting para fechá-la automaticamente antes de recompilar." -ForegroundColor DarkGray
        }
    }

    $distPath = Join-Path $projectRoot 'dist'
    if ($Clean -and (Test-Path -LiteralPath $distPath)) {
        Write-Host "Limpando diretório dist..." -ForegroundColor Cyan
        $checkedDist = [System.IO.Path]::GetFullPath($distPath)
        if ($checkedDist -ne (Join-Path $projectRoot 'dist') -or (Get-Item -LiteralPath $distPath).LinkType) { throw 'Refusing to clean an unexpected or linked directory.' }
        Remove-Item -LiteralPath $checkedDist -Recurse -Force
    }

    if (-not $SkipChecks) {
        Write-Host "Executando verificações de qualidade (fmt, test, clippy)..." -ForegroundColor Cyan
        & cargo fmt --check
        if ($LASTEXITCODE -ne 0) { throw 'cargo fmt --check failed.' }
        & cargo test --locked
        if ($LASTEXITCODE -ne 0) { throw 'cargo test failed.' }
        & cargo clippy --locked --all-targets -- -D warnings
        if ($LASTEXITCODE -ne 0) { throw 'cargo clippy failed.' }
    }

    Write-Host "Compilando binário em modo Release..." -ForegroundColor Cyan
    & cargo build --release --locked
    if ($LASTEXITCODE -ne 0) { throw 'Release build failed.' }

    New-Item -ItemType Directory -Path $distPath -Force | Out-Null
    $binaryPath = Join-Path $projectRoot 'target\release\nivumi.exe'
    $portablePath = Join-Path $distPath 'nivumi.exe'
    Copy-Item -LiteralPath $binaryPath -Destination $portablePath -Force
    $portableHash = Get-FileHash -LiteralPath $portablePath -Algorithm SHA256
    "$($portableHash.Hash.ToLowerInvariant())  nivumi.exe" | Set-Content -LiteralPath (Join-Path $distPath 'SHA256SUMS.txt') -Encoding ascii
    Write-Output "Portable executable: $portablePath"

    $portableDirectory = Join-Path $distPath 'Nivumi-Portable'
    New-Item -ItemType Directory -Path $portableDirectory -Force | Out-Null
    Copy-Item -LiteralPath $binaryPath -Destination (Join-Path $portableDirectory 'nivumi.exe') -Force
    Set-Content -LiteralPath (Join-Path $portableDirectory 'portable.flag') -Value '' -Encoding ascii
    @'
Nivumi - Portátil (Windows x64)

Extraia o ZIP em uma pasta com permissão de escrita e abra nivumi.exe.
Mantenha portable.flag ao lado do executável. Os dados ficam em data\state.json.
Para transportar configurações, feche o aplicativo e copie a pasta inteira.
NIVUMI_DATA_DIR (ou o alias NUSA_REBUILD_DATA_DIR) tem prioridade sobre o modo portátil.
Os arquivos da Prateleira continuam sendo referências aos caminhos originais.
Ativar a inicialização com o Windows registra o caminho atual neste computador;
desative essa opção antes de mover a pasta. Não é necessário instalar.
'@ | Set-Content -LiteralPath (Join-Path $portableDirectory 'LEIA-ME.txt') -Encoding utf8

    if (-not $NoZip) {
        $zipPath = Join-Path $distPath 'Nivumi-Portable.zip'
        $portableFiles = @('nivumi.exe', 'portable.flag', 'LEIA-ME.txt') | ForEach-Object { Join-Path $portableDirectory $_ }
        Compress-Archive -LiteralPath $portableFiles -DestinationPath $zipPath -Force
        $zipHash = Get-FileHash -LiteralPath $zipPath -Algorithm SHA256
        "$($zipHash.Hash.ToLowerInvariant())  Nivumi-Portable.zip" | Add-Content -LiteralPath (Join-Path $distPath 'SHA256SUMS.txt') -Encoding ascii
        Write-Output "Portable ZIP: $zipPath"
    }

    if ($Installer) {
        Write-Host "Compilando instalador Inno Setup..." -ForegroundColor Cyan
        if (-not $IsccPath) {
            $isccCommand = Get-Command ISCC.exe -ErrorAction SilentlyContinue
            if ($isccCommand) { $IsccPath = $isccCommand.Source }
        }
        if (-not $IsccPath) {
            $isccCandidates = @(
                'C:\Program Files (x86)\Inno Setup 6\ISCC.exe',
                'C:\Program Files\Inno Setup 6\ISCC.exe',
                'C:\Program Files (x86)\Inno Setup 7\ISCC.exe',
                'C:\Program Files\Inno Setup 7\ISCC.exe',
                (Join-Path $projectRoot 'analysis\tools\inno-6.7.0\{app}\ISCC.exe')
            )
            $IsccPath = $isccCandidates | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
        }
        if (-not $IsccPath -or -not (Test-Path -LiteralPath $IsccPath)) {
            throw 'Inno Setup compiler not found. The portable executable was built. Provide -IsccPath or NUSA_ISCC_PATH to create the installer; this script does not install tools.'
        }
        $metadata = & cargo metadata --no-deps --format-version 1
        if ($LASTEXITCODE -ne 0) { throw 'Unable to read Cargo metadata.' }
        $package = ($metadata | ConvertFrom-Json).packages | Where-Object { $_.name -eq 'nivumi' } | Select-Object -First 1
        & $IsccPath "/DAppVersion=$($package.version)" (Join-Path $projectRoot 'packaging\installer.iss')
        if ($LASTEXITCODE -ne 0) { throw 'Inno Setup compilation failed.' }
        $setupPath = Join-Path $distPath 'NivumiSetup.exe'
        $setupHash = Get-FileHash -LiteralPath $setupPath -Algorithm SHA256
        "$($setupHash.Hash.ToLowerInvariant())  NivumiSetup.exe" | Add-Content -LiteralPath (Join-Path $distPath 'SHA256SUMS.txt') -Encoding ascii
        Write-Output "Installer: $setupPath"
    }

    if ($Run) {
        Write-Host "Iniciando executável portátil ($portablePath)..." -ForegroundColor Green
        $proc = Start-Process -FilePath $portablePath -WorkingDirectory $distPath -PassThru
        Write-Host "Nivumi iniciado com sucesso (PID $($proc.Id))." -ForegroundColor Green
    }
} finally {
    Pop-Location
}
