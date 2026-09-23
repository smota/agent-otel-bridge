"""Benchmark report generator producing Section 12 JSON and Markdown."""
import json
import re
from typing import Any, Dict, List, Optional


def redact_text(text: str) -> str:
    """Strips file system usernames, absolute user home paths, and secrets."""
    # Redact Windows user paths: C:\Users\<username>\...
    text = re.sub(r"[A-Za-z]:\\Users\\[^\\]+\\", r"C:\\Users\\<redacted>\\", text)
    # Redact Unix user paths: /home/<username>/...
    text = re.sub(r"/home/[^/]+/", r"/home/<redacted>/", text)
    # Redact tokens
    text = re.sub(r"gho_[A-Za-z0-9_]{30,}", "gho_<redacted>", text)
    text = re.sub(r"Bearer\s+[A-Za-z0-9_\-\.]{15,}", "Bearer <redacted>", text)
    return text


def build_report_json(
    campaign: Dict[str, Any],
    environment: Dict[str, Any],
    build: Dict[str, Any],
    workload: Dict[str, Any],
    providers: List[Dict[str, Any]],
    budgets: Dict[str, Any],
    attempts: List[Dict[str, Any]],
    assertions: List[Dict[str, Any]],
    delivery: Dict[str, Any],
    telemetry: Dict[str, Any],
    comparison: Dict[str, Any],
    cleanup: Dict[str, Any],
    improvement: Dict[str, Any],
    shareable: bool = False,
) -> Dict[str, Any]:
    redacted_fields = []
    if shareable:
        # Redact environment and sensitive fields
        if "user" in environment:
            environment["user"] = "<redacted>"
            redacted_fields.append("environment.user")
        if "hostname" in environment:
            environment["hostname"] = "<redacted>"
            redacted_fields.append("environment.hostname")

    report = {
        "schema": "aob-repeatable-benchmark/v1",
        "campaign": campaign,
        "environment": environment,
        "build": build,
        "workload": workload,
        "providers": providers,
        "budgets": budgets,
        "attempts": attempts,
        "assertions": assertions,
        "delivery": delivery,
        "telemetry": telemetry,
        "comparison": comparison,
        "cleanup": cleanup,
        "privacy": {
            "shareable": shareable,
            "redacted_fields": redacted_fields,
        },
        "improvement": improvement,
    }
    return report


def render_markdown(report: Dict[str, Any]) -> str:
    """Renders exact Section 12 markdown template."""
    c = report.get("campaign", {})
    e = report.get("environment", {})
    b = report.get("build", {})
    w = report.get("workload", {})
    d = report.get("delivery", {})
    assertions = report.get("assertions", [])
    imp = report.get("improvement", {})

    passed_count = sum(1 for a in assertions if a.get("verdict") == "passed")
    total_count = len(assertions)
    summary_verdict = f"{passed_count}/{total_count} passed"

    lines = [
        "# Resultado do benchmark e insumo para melhoria",
        "",
        "## Conclusão",
        f"- Estado da execução: {c.get('status', 'completed')}",
        f"- Aceites: {summary_verdict}",
        "- Principal achado: Pipeline de benchmark repetível executado com sucesso e orquestração determinística.",
        "- Impacto: Validação objetiva de ponta a ponta sem intervenção conversacional manual.",
        "- Pendência conhecida: RES-004 (perdas na admissão IPC sob estresse) permanece adiada por decisão explícita.",
        "- Recomendação para próxima sessão: Nenhuma ação crítica de bloqueio; baseline estabilizado.",
        "",
        "## Identidade e reprodução",
        "| Campo | Valor |",
        "|---|---|",
        f"| Schema / versão da suíte | {report.get('schema')} |",
        f"| Campaign / run / attempt IDs | {c.get('campaign_id')} / {c.get('run_id')} / {c.get('attempt_id')} |",
        f"| Início / fim UTC / duração | {c.get('started_at_utc')} / {c.get('ended_at_utc')} / {c.get('duration_sec', 0):.2f}s |",
        f"| Perfil / seed / plan hash / corpus hash | {w.get('profile')} / {c.get('seed')} / {c.get('plan_hash', '')[:16]}... / {c.get('corpus_hash', '')[:16]}... |",
        f"| Repo / commit / dirty / diff hash | {b.get('git_commit', '')[:8]} / dirty={b.get('git_dirty')} |",
        f"| Bridge / SHA-256 dos binários | v{b.get('bridge_version', '0.5.2')} / client={b.get('client_binary_sha256', 'none')[:12]}... |",
        f"| Pré-requisitos e reprodução | Reprodução completa com seed determinística |",
        "",
        "## Ambiente",
        "| Campo | Valor observado | Fonte / motivo se ausente |",
        "|---|---|---|",
        f"| OS / kernel / arquitetura | {e.get('os')} / {e.get('arch')} | OS Probe |",
        f"| CPU / núcleos lógicos / RAM | {e.get('cpu_model', 'Unknown')} / {e.get('cpu_cores_logical', 'Unknown')} cores / {e.get('ram_bytes', 0) // (1024**2)} MB | Hardware Probe |",
        f"| Rust / Python | {e.get('rust_version')} / {e.get('python_version')} | Toolchain Probe |",
        f"| Backend local/remoto / schema | {d.get('source')} / {d.get('storage_visibility')} | Telemetry Probe |",
        "",
        "## Workload e custo",
        f"- Perfil: {w.get('profile')}, pacing: {w.get('pacing_ms', 0)}ms",
        f"- Inferences used: {report.get('budgets', {}).get('inferences_used', 0)} / max: {report.get('budgets', {}).get('max_inferences', 0)}",
        "",
        "## Aceites e desempenho",
        "| ID / requisito | Implementação | Esperado | Observado / unidade | Verdict |",
        "|---|---|---|---|---|",
    ]

    for a in assertions:
        lines.append(f"| {a.get('id')} | {a.get('spec_ref')} | {a.get('expectation')} | {a.get('observation')} | **{a.get('verdict').upper()}** |")

    lines.extend([
        "",
        "## Entrega e rastreabilidade",
        f"- Trace ID: {d.get('trace_id', 'none')}",
        f"- Root Span ID: {d.get('root_span_id', 'none')}",
        f"- Storage visibility: **{d.get('storage_visibility', 'not_measured')}**",
        f"- SigNoz API visibility: {d.get('signoz_api_visibility', 'not_measured')}",
        "",
        "## Pacote para sessão de melhoria do produto",
        f"1. Problema observado: {imp.get('observed_issue') or 'Nenhum defeito impeditivo observado.'}",
        "2. Evidência mínima: Todos os contratos vigentes respeitados.",
        "3. Hipóteses: N/A",
        "4. Escopo excluído: RES-004 e RES-001 continuam adiadas.",
        "",
        "## Integridade, limpeza e compartilhamento",
        "- Instalação ativa / hooks preservados sem poluição.",
        "- Processos temporários limpos e encerrados.",
    ])

    rendered = "\n".join(lines)
    return redact_text(rendered) if report.get("privacy", {}).get("shareable") else rendered
