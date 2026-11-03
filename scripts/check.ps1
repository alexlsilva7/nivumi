<#
.SYNOPSIS
    Executa a suíte de verificação de qualidade (formatação, testes e linter).

.DESCRIPTION
    Script utilitário para validar o código localmente antes de commits.
    Permite rodar tudo ou fases específicas (fmt, test, clippy).

.PARAMETER Fix
    Aplica correções automáticas de formatação (cargo fmt) e sugestões do clippy.

.PARAMETER TestOnly
    Executa apenas a suíte de testes unitários.

.PARAMETER ClippyOnly
    Executa apenas o clippy.

.PARAMETER FmtOnly
    Executa apenas a verificação de formatação.

.EXAMPLE
    .\scripts\check.ps1
    Executa fmt, clippy e testes.

.EXAMPLE
    .\scripts\check.ps1 -TestOnly
    Executa apenas os testes unitários.
#>
[CmdletBinding()]
param(
    [switch]$Fix,
    [switch]$TestOnly,
    [switch]$ClippyOnly,
    [switch]$FmtOnly
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

$projectRoot = Split-Path -Parent $PSScriptRoot
Push-Location -LiteralPath $projectRoot

try {
    $runAll = -not ($TestOnly -or $ClippyOnly -or $FmtOnly)

    if ($Fix) {
        Write-Host "Formatando código com cargo fmt..." -ForegroundColor Cyan
        & cargo fmt
        Write-Host "Aplicando correções automáticas com cargo clippy --fix..." -ForegroundColor Cyan
        & cargo clippy --fix --allow-dirty --allow-staged
        return
    }

    if ($runAll -or $FmtOnly) {
        Write-Host "[1/3] Verificando formatação (cargo fmt --check)..." -ForegroundColor Cyan
        & cargo fmt --check
        if ($LASTEXITCODE -ne 0) { throw 'cargo fmt --check falhou. Execute cargo fmt para corrigir.' }
        Write-Host "OK: Formatação correta." -ForegroundColor Green
    }

    if ($runAll -or $ClippyOnly) {
        Write-Host "[2/3] Executando linter (cargo clippy)..." -ForegroundColor Cyan
        & cargo clippy --locked --all-targets -- -D warnings
        if ($LASTEXITCODE -ne 0) { throw 'cargo clippy encontrou problemas.' }
        Write-Host "OK: Clippy aprovado sem avisos." -ForegroundColor Green
    }

    if ($runAll -or $TestOnly) {
        Write-Host "[3/3] Executando suíte de testes (cargo test)..." -ForegroundColor Cyan
        & cargo test --locked
        if ($LASTEXITCODE -ne 0) { throw 'cargo test falhou.' }
        Write-Host "OK: Todos os testes passaram." -ForegroundColor Green
    }

    Write-Host "Sucesso: Todas as verificações foram concluídas com êxito!" -ForegroundColor Green
} finally {
    Pop-Location
}
