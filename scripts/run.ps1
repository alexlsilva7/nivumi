<#
.SYNOPSIS
    Compila e executa o Nivumi.

.DESCRIPTION
    Script de desenvolvimento e teste rápido. Compila o projeto (Debug por padrão ou Release)
    e inicia o executável imediatamente, com opções úteis para inspeção e testes visuais.

.PARAMETER Release
    Compila em modo Release (otimizado). Por padrão, compila em modo Debug (compilação incremental mais rápida).

.PARAMETER KillExisting
    Fecha automaticamente qualquer instância do nivumi.exe que já esteja em execução
    para evitar bloqueio de arquivo durante o link do compilador.

.PARAMETER Inspect
    Inicia o aplicativo com a flag --inspect (mantém a notch fixada e sempre visível para inspeção).

.PARAMETER Expanded
    Inicia o aplicativo diretamente no modo expandido (--expanded).

.PARAMETER Page
    Inicia o aplicativo diretamente na página especificada:
    0 = Player de Mídia
    1 = Letras Sincronizadas
    2 = Pomodoro
    3 = Hidratação
    4 = Prateleira
    5 = Tarefas
    6 = Calendário

.PARAMETER Wait
    Aguarda a finalização do processo no console em vez de liberar o terminal imediatamente.

.PARAMETER Check
    Executa `cargo check` antes da compilação.

.PARAMETER AppArgs
    Argumentos adicionais repassados diretamente ao executável.

.EXAMPLE
    .\scripts\run.ps1
    Compila em modo Debug e executa o aplicativo.

.EXAMPLE
    .\scripts\run.ps1 -Release
    Compila em Release e executa.

.EXAMPLE
    .\scripts\run.ps1 -Inspect -Expanded -Page 0
    Abre o app expandido no player de mídia com a notch fixada no topo.

.EXAMPLE
    .\scripts\run.ps1 -KillExisting
    Encerra qualquer instância anterior do app e inicia a nova versão.
#>
[CmdletBinding()]
param(
    [switch]$Release,
    [switch]$KillExisting,
    [switch]$Inspect,
    [switch]$Expanded,
    [int]$Page = -1,
    [switch]$Wait,
    [switch]$Check,
    [string[]]$AppArgs
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$projectRoot = Split-Path -Parent $PSScriptRoot
Push-Location -LiteralPath $projectRoot

try {
    # 1. Tratar instâncias anteriores em execução
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

    # 2. Verificação rápida opcional
    if ($Check) {
        Write-Host "Executando cargo check..." -ForegroundColor Cyan
        & cargo check
        if ($LASTEXITCODE -ne 0) { throw "cargo check falhou com código $LASTEXITCODE." }
    }

    # 3. Compilação
    $buildConfig = if ($Release) { 'release' } else { 'debug' }
    Write-Host "Compilando Nivumi (modo $buildConfig)..." -ForegroundColor Cyan

    if ($Release) {
        & cargo build --release
    } else {
        & cargo build
    }
    if ($LASTEXITCODE -ne 0) { throw "Falha na compilação ($buildConfig). Verifique os erros acima." }

    # 4. Localizar o executável gerado
    $binaryPath = Join-Path $projectRoot "target\$buildConfig\nivumi.exe"
    if (-not (Test-Path -LiteralPath $binaryPath)) {
        throw "Executável não encontrado em: $binaryPath"
    }

    # 5. Montar argumentos do aplicativo
    $arguments = [System.Collections.Generic.List[string]]::new()
    if ($Inspect) {
        $arguments.Add('--inspect')
    }
    if ($Expanded) {
        $arguments.Add('--expanded')
    }
    if ($Page -ge 0) {
        $arguments.Add("--page=$Page")
    }
    if ($AppArgs) {
        foreach ($arg in $AppArgs) {
            $arguments.Add($arg)
        }
    }

    $argString = ($arguments | ForEach-Object { if ($_ -match '\s') { "`"$_`"" } else { $_ } }) -join ' '

    # 6. Execução
    Write-Host "Iniciando $binaryPath $(if ($argString) { "com argumentos: $argString" })..." -ForegroundColor Green

    $splat = @{
        FilePath = $binaryPath
        WorkingDirectory = $projectRoot
        PassThru = $true
    }
    if ($arguments.Count -gt 0) {
        $splat['ArgumentList'] = $arguments.ToArray()
    }

    if ($Wait) {
        $splat['Wait'] = $true
        $proc = Start-Process @splat
        Write-Host "Processo finalizado com código de saída: $($proc.ExitCode)" -ForegroundColor Gray
    } else {
        $proc = Start-Process @splat
        Write-Host "Nivumi iniciado com sucesso (PID $($proc.Id))." -ForegroundColor Green
    }
} finally {
    Pop-Location
}
