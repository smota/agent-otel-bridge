<#
.SYNOPSIS
    Automated Guardrail & Quality Verification Tool for agent-otel-bridge.
    Enforces design constraints, performance thresholds, formatting, and tests.
#>

param(
    [switch]$Quick,
    [switch]$Fix
)

$ErrorActionPreference = "Continue"
$sw = [System.Diagnostics.Stopwatch]::StartNew()

Write-Host "============================================================" -ForegroundColor Cyan
Write-Host "  agent-otel-bridge: Automated Guardrail Verification Tool  " -ForegroundColor Cyan
Write-Host "============================================================" -ForegroundColor Cyan
Write-Host ""

$failures = 0

function Run-Guardrail([string]$Name, [scriptblock]$Action) {
    Write-Host -NoNewline "[GUARDRAIL] $Name... "
    $output = & $Action
    if ($LASTEXITCODE -eq 0) {
        Write-Host "PASSED" -ForegroundColor Green
    } else {
        Write-Host "FAILED" -ForegroundColor Red
        if ($output) {
            Write-Host ($output | Out-String) -ForegroundColor Yellow
        }
        $script:failures++
    }
}

# 1. Code Formatting
Run-Guardrail "Checking code formatting (cargo fmt)" {
    if ($Fix) {
        cargo fmt
    }
    cargo fmt --check
}

# 2. Clippy Linter & Warnings
Run-Guardrail "Checking linter purity (cargo clippy --all-targets)" {
    cargo clippy --workspace --all-targets -- -D warnings
}

# 3. Workspace Test Suite
Run-Guardrail "Running workspace test suite (cargo test --workspace)" {
    cargo test --workspace
}

# 4. Documentation Generation
Run-Guardrail "Validating documentation generation (cargo doc --no-deps)" {
    cargo doc --workspace --no-deps
}

# 5. Client Binary Size SLA Check (< 350 KB)
Run-Guardrail "Checking client binary size SLA (< 350 KB)" {
    cargo build --release -p agent-otel-client
    $binPath = "target/release/agent-hook.exe"
    if (Test-Path $binPath) {
        $sizeBytes = (Get-Item $binPath).Length
        $sizeKB = [math]::Round($sizeBytes / 1024, 1)
        if ($sizeKB -gt 350) {
            Write-Error "agent-hook.exe size ($sizeKB KB) exceeds 350 KB SLA!"
            return
        }
        Write-Host -NoNewline "(${sizeKB} KB) " -ForegroundColor DarkGray
    }
}

# 6. Branch Naming Policy Check (if on git branch)
Run-Guardrail "Validating Git branch naming policy" {
    $branch = (git rev-parse --abbrev-ref HEAD 2>$null)
    if ($branch) {
        $branch = $branch.Trim()
        $allowed = ($branch -eq "main") -or 
                   ($branch -eq "master") -or 
                   ($branch -like "feat/*") -or 
                   ($branch -like "fix/*") -or 
                   ($branch -like "docs/*") -or 
                   ($branch -like "perf/*") -or 
                   ($branch -like "release/*")
        if (-not $allowed) {
            Write-Host -NoNewline "($branch) " -ForegroundColor DarkYellow
            Write-Host -NoNewline "Branch should follow 'feat/*', 'fix/*', 'docs/*', or 'main' - " -ForegroundColor DarkYellow
        }
    }
}

$sw.Stop()
$elapsedSec = [math]::Round($sw.Elapsed.TotalSeconds, 1)

Write-Host ""
Write-Host "============================================================" -ForegroundColor Cyan
if ($failures -eq 0) {
    Write-Host "  ALL GUARDRAILS PASSED (Elapsed: ${elapsedSec}s)          " -ForegroundColor Green
    Write-Host "============================================================" -ForegroundColor Cyan
    exit 0
} else {
    Write-Host "  $failures GUARDRAIL CHECK(S) FAILED (Elapsed: ${elapsedSec}s) " -ForegroundColor Red
    Write-Host "============================================================" -ForegroundColor Cyan
    exit 1
}
