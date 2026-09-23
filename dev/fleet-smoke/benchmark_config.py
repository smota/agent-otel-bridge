"""Benchmark configuration loader and plan resolver."""
from dataclasses import dataclass, field
import hashlib
import json
import os
from pathlib import Path
import sys
import time
import uuid
from typing import Any, Dict, Optional

HERE = Path(__file__).resolve().parent
SCHEMAS_DIR = HERE / "schemas"
PROFILES_FILE = HERE / "profiles" / "repeatable-v1.json"


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def sha256_file(path: Path) -> Optional[str]:
    try:
        with open(path, "rb") as f:
            return hashlib.file_digest(f, "sha256").hexdigest()
    except OSError:
        return None


@dataclass
class ResolvedPlan:
    schema_version: str
    campaign_id: str
    run_id: str
    attempt_id: str
    profile: str
    seed: int
    started_at_utc: str
    limits: Dict[str, Any]
    backend: Dict[str, Any]
    candidate_paths: Dict[str, Any]
    provider_programs: Dict[str, Any]
    models: Dict[str, Any]
    plan_hash: str = ""
    corpus_hash: str = ""

    def to_dict(self) -> Dict[str, Any]:
        return {
            "schema_version": self.schema_version,
            "campaign_id": self.campaign_id,
            "run_id": self.run_id,
            "attempt_id": self.attempt_id,
            "profile": self.profile,
            "seed": self.seed,
            "started_at_utc": self.started_at_utc,
            "limits": self.limits,
            "backend": self.backend,
            "candidate_paths": self.candidate_paths,
            "provider_programs": self.provider_programs,
            "models": self.models,
            "plan_hash": self.plan_hash,
            "corpus_hash": self.corpus_hash,
        }


def load_profile_defaults(profile_name: str) -> Dict[str, Any]:
    if PROFILES_FILE.exists():
        with open(PROFILES_FILE, "r", encoding="utf-8") as f:
            data = json.load(f)
            profiles = data.get("profiles", {})
            if profile_name in profiles:
                return profiles[profile_name]

    # Fallback inline defaults
    if profile_name == "quick":
        return {"max_inferences": 0, "global_timeout_sec": 900, "pacing_ms": 100}
    elif profile_name == "stress":
        return {"max_inferences": 0, "global_timeout_sec": 1800, "pacing_ms": 0}
    else:
        return {"max_inferences": 3, "global_timeout_sec": 1200, "pacing_ms": 500}


def resolve_plan(
    config_path: Optional[str] = None,
    profile: str = "quick",
    seed: Optional[int] = None,
) -> ResolvedPlan:
    config: Dict[str, Any] = {}
    if config_path and Path(config_path).exists():
        with open(config_path, "r", encoding="utf-8") as f:
            config = json.load(f)

    profile_name = config.get("profile", profile)
    profile_defaults = load_profile_defaults(profile_name)

    resolved_seed = int(config.get("seed", seed if seed is not None else 502))

    limits = {
        "global_timeout_sec": int(config.get("limits", {}).get("global_timeout_sec", profile_defaults.get("global_timeout_sec", 900))),
        "max_inferences": int(config.get("limits", {}).get("max_inferences", profile_defaults.get("max_inferences", 0))),
        "per_call_timeout_sec": int(config.get("limits", {}).get("per_call_timeout_sec", 90)),
        "max_output_tokens": int(config.get("limits", {}).get("max_output_tokens", 1024)),
        "max_artifacts_bytes": int(config.get("limits", {}).get("max_artifacts_bytes", 268435456)),
    }

    backend = config.get("backend", {
        "type": "clickhouse_direct",
        "endpoint": os.environ.get("AOB_CLICKHOUSE_URL", "http://localhost:8123"),
        "database": os.environ.get("AOB_CLICKHOUSE_DB", "signoz_traces"),
        "table": "distributed_signoz_index_v3",
        "schema_version": "v3",
        "tenant_id": None,
        "credentials_env_var": "AOB_CLICKHOUSE_CREDENTIALS",
        "trace_url_template": os.environ.get("AOB_SIGNOZ_TRACE_URL_TEMPLATE"),
    })

    candidate_paths = config.get("candidate_paths", {
        "client": str(HERE.parent.parent / "target" / "release" / "agent-hook.exe"),
        "daemon": str(HERE.parent.parent / "target" / "release" / "agent-otel-bridge.exe"),
        "cli": str(HERE.parent.parent / "target" / "release" / "agent-otel-bridge.exe"),
    })

    provider_programs = config.get("provider_programs", {
        "codex": "codex",
        "grok": "grok",
        "antigravity": "agy",
    })

    models = config.get("models", {
        "codex": "gpt-5.6-luna",
        "grok": "grok-4.5",
        "antigravity": "gemini-3.8-flash-low",
    })

    # Fresh unique IDs per run
    campaign_id = f"cmp-{uuid.uuid4().hex[:12]}"
    run_id = f"run-{uuid.uuid4().hex[:16]}"
    attempt_id = f"att-1"
    now_utc = time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())

    # Deterministic corpus hash from prompts directory
    prompts_dir = HERE / "prompts"
    prompt_hashes = []
    if prompts_dir.exists():
        for p in sorted(prompts_dir.glob("*.txt")):
            h = sha256_file(p)
            if h:
                prompt_hashes.append(f"{p.name}:{h}")
    corpus_hash = sha256_bytes("|".join(prompt_hashes).encode("utf-8")) if prompt_hashes else sha256_bytes(b"empty")

    plan = ResolvedPlan(
        schema_version="aob-benchmark-config/v1",
        campaign_id=campaign_id,
        run_id=run_id,
        attempt_id=attempt_id,
        profile=profile_name,
        seed=resolved_seed,
        started_at_utc=now_utc,
        limits=limits,
        backend=backend,
        candidate_paths=candidate_paths,
        provider_programs=provider_programs,
        models=models,
        corpus_hash=corpus_hash,
    )

    # Compute stable plan hash
    plan_data = json.dumps(plan.to_dict(), sort_keys=True).encode("utf-8")
    plan.plan_hash = sha256_bytes(plan_data)

    return plan
