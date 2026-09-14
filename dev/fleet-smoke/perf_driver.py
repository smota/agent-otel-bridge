import argparse
import datetime
import hashlib
import json
import math
import os
import platform
import subprocess
import sys
from typing import Any, Dict, List, Optional, Set

PARSER_THROUGHPUT_MIN_SPS = 50000.0
HARVEST_LATENCY_MAX_US = 150.0
HOOK_BINARY_SIZE_MAX_BYTES = 300_000  # Conservative decimal interpretation of <300 KB.
LEGACY_HOOK_BINARY_SIZE_MAX_BYTES = 300 * 1024
IPC_P99_MAX_US = 3000.0
HOOK_INTERNAL_MAX_US = 1000.0

REQUIRED_WINDOWS_OBSERVATIONS = [
    "hook_internal_execution_duration_us",
    "ipc_roundtrip_p99_us",
    "concurrent_event_delivery_loss",
]

OUT_OF_SCOPE_PLATFORMS = [
    "linux_native_performance",
    "macos_native_performance",
]

NORMATIVE_REF_COORDINATION = "dev/fleet-smoke/performance-coordination.md#required-measurements-and-assertions"
NORMATIVE_REF_AGENTS = "AGENTS.md#2-performance-slas--hard-boundaries"


def sanitize_for_json(val: Any) -> Any:
    if isinstance(val, float):
        if math.isnan(val) or math.isinf(val):
            return str(val)
        return val
    if isinstance(val, dict):
        return {k: sanitize_for_json(v) for k, v in val.items()}
    if isinstance(val, list):
        return [sanitize_for_json(v) for v in val]
    return val


def compute_sha256(file_path: str) -> Optional[str]:
    if not os.path.isfile(file_path):
        return None
    hasher = hashlib.sha256()
    with open(file_path, "rb") as f:
        while chunk := f.read(65536):
            hasher.update(chunk)
    return hasher.hexdigest()


def evaluate_metric_threshold(
    value: Optional[float],
    threshold: float,
    mode: str,
    metric_name: str,
    normative_ref: str,
    impl_ref: str,
    unit: str,
) -> Dict[str, Any]:
    if mode not in ("strictly_greater", "strictly_less"):
        return {
            "verdict": "failed",
            "metric": metric_name,
            "reason": f"invalid_evaluation_mode: {mode}",
            "normative_ref": normative_ref,
            "impl_ref": impl_ref,
            "unit": unit,
            "threshold": threshold,
            "value": sanitize_for_json(value),
        }

    base = {
        "metric": metric_name,
        "normative_ref": normative_ref,
        "impl_ref": impl_ref,
        "unit": unit,
        "threshold": threshold,
        "mode": mode,
    }

    if value is None:
        base.update({"verdict": "not_measured", "reason": "value_absent", "value": None})
        return base

    if (
        isinstance(value, bool)
        or not isinstance(value, (int, float))
        or math.isnan(value)
        or math.isinf(value)
        or value < 0.0
    ):
        base.update({
            "verdict": "failed",
            "reason": "invalid_nonfinite_or_negative_value",
            "value": sanitize_for_json(value),
        })
        return base

    passed = (value > threshold) if mode == "strictly_greater" else (value < threshold)
    base.update({
        "verdict": "passed" if passed else "failed",
        "value": value,
    })
    return base


def collect_environment() -> Dict[str, Any]:
    rustc_ver = None
    try:
        res = subprocess.run(["rustc", "--version"], capture_output=True, text=True, timeout=5)
        if res.returncode == 0:
            rustc_ver = res.stdout.strip()
    except Exception as exc:
        rustc_ver = f"unavailable: {exc}"

    return {
        "timestamp_utc": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "os": platform.system(),
        "os_release": platform.release(),
        "os_version": platform.version(),
        "architecture": platform.machine(),
        "processor": platform.processor(),
        "cpu_count": os.cpu_count(),
        "python_version": platform.python_version(),
        "rustc_version": rustc_ver,
        "out_of_scope_platforms": [
            {
                "platform": p,
                "verdict": "not_measured",
                "reason": "unavailable_native_host",
                "outside_required_scope": True,
            }
            for p in OUT_OF_SCOPE_PLATFORMS
        ],
    }


def inspect_source_state(repo_root: str = ".") -> Dict[str, Any]:
    rev = None
    dirty = None
    diff_hash = None
    untracked_hashes: Dict[str, str] = {}
    untracked_source_hashes: Dict[str, str] = {}

    try:
        r = subprocess.run(["git", "rev-parse", "HEAD"], cwd=repo_root, capture_output=True, text=True, timeout=5)
        if r.returncode == 0:
            rev = r.stdout.strip()

        diff_proc = subprocess.run(["git", "diff", "HEAD"], cwd=repo_root, capture_output=True, timeout=10)
        if diff_proc.returncode == 0:
            diff_hash = hashlib.sha256(diff_proc.stdout).hexdigest()

        st_proc = subprocess.run(["git", "status", "--porcelain"], cwd=repo_root, capture_output=True, text=True, timeout=5)
        if st_proc.returncode == 0:
            dirty = len(st_proc.stdout.strip()) > 0
        untracked_proc = subprocess.run(
            ["git", "ls-files", "--others", "--exclude-standard", "-z"],
            cwd=repo_root, capture_output=True, timeout=5,
        )
        if untracked_proc.returncode != 0:
            raise RuntimeError("cannot enumerate untracked candidate sources")
        for raw_path in untracked_proc.stdout.split(b"\0"):
            if raw_path:
                relative = os.fsdecode(raw_path)
                digest = compute_sha256(os.path.join(repo_root, relative))
                if digest is None:
                    raise RuntimeError("cannot hash untracked candidate source")
                untracked_source_hashes[relative.replace("\\", "/")] = digest
    except Exception as exc:
        dirty = f"error: {exc}"

    fleet_smoke_dir = os.path.join(repo_root, "dev", "fleet-smoke")
    if os.path.isdir(fleet_smoke_dir):
        for root, dirs, files in os.walk(fleet_smoke_dir):
            dirs[:] = [d for d in dirs if d != "__pycache__" and not d.startswith(".")]
            for file in files:
                if file.endswith(".pyc") or file.endswith(".pyo"):
                    continue
                fpath = os.path.join(root, file)
                rel = os.path.relpath(fpath, repo_root)
                file_hash = compute_sha256(fpath)
                if file_hash:
                    untracked_hashes[rel.replace("\\", "/")] = file_hash

    return {
        "installed_reference_revision": "a27470c",
        "candidate_git_revision": rev,
        "is_dirty": dirty,
        "git_diff_head_sha256": diff_hash,
        "dev_fleet_smoke_source_sha256": untracked_hashes,
        "untracked_source_sha256": untracked_source_hashes,
    }


def inspect_binary(path: str, is_hook_binary: bool = False, build_profile: Optional[str] = None) -> Dict[str, Any]:
    if not os.path.exists(path):
        return {"path": path, "exists": False, "provenance": "unknown_absent"}

    size_bytes = os.path.getsize(path)
    sha256 = compute_sha256(path)
    provenance = f"profile_{build_profile}" if build_profile else "unknown_unspecified_profile"

    meta: Dict[str, Any] = {
        "path": path,
        "exists": True,
        "size_bytes": size_bytes,
        "sha256": sha256,
        "provenance": "unverified_prebuilt_source_mapping",
        "declared_build_profile": provenance,
    }

    if is_hook_binary:
        meta["legacy_binary_size_reference"] = {
            "threshold_bytes": LEGACY_HOOK_BINARY_SIZE_MAX_BYTES,
            "below_threshold": size_bytes < LEGACY_HOOK_BINARY_SIZE_MAX_BYTES,
            "acceptance_proxy": False,
        }
        meta["size_evaluation"] = evaluate_metric_threshold(
            float(size_bytes),
            float(HOOK_BINARY_SIZE_MAX_BYTES),
            "strictly_less",
            "hook_binary_size",
            NORMATIVE_REF_COORDINATION,
            "crates/agent-otel-client",
            "bytes",
        )

    return meta


def default_active_manifest_path() -> Optional[str]:
    local_app_data = os.environ.get("LOCALAPPDATA")
    if not local_app_data:
        return None
    candidate = os.path.join(local_app_data, "agent-otel-bridge", "active.json")
    return candidate


def verify_active_manifest(manifest_path: str) -> Dict[str, Any]:
    if not manifest_path or not os.path.isfile(manifest_path):
        return {
            "verified": False,
            "manifest_path": manifest_path,
            "reason": "manifest_file_not_found",
        }

    manifest_hash_before = compute_sha256(manifest_path)
    try:
        with open(manifest_path, "r", encoding="utf-8") as f:
            data = json.load(f)
    except Exception as exc:
        return {
            "verified": False,
            "manifest_path": manifest_path,
            "reason": f"json_parse_error: {exc}",
        }

    if not isinstance(data, dict):
        return {"verified": False, "reason": "manifest_root_not_object"}

    version = data.get("version")
    git_commit = data.get("git_commit")
    version_id = data.get("version_id")
    binaries = data.get("binaries")

    if not version or not git_commit or not version_id or not isinstance(binaries, list) or len(binaries) == 0:
        return {
            "verified": False,
            "reason": "invalid_or_empty_schema_fields",
            "details": {"version": version, "git_commit": git_commit, "binaries_type": type(binaries).__name__},
        }

    manifest_dir = os.path.dirname(os.path.abspath(manifest_path))
    bin_dir = os.path.abspath(os.path.join(manifest_dir, "bin"))

    seen_filenames: Set[str] = set()
    binary_results = []
    all_valid = True

    for entry in binaries:
        if not isinstance(entry, dict):
            all_valid = False
            binary_results.append({"entry": str(entry), "valid": False, "reason": "entry_not_object"})
            continue

        filename = entry.get("filename")
        exp_sha = entry.get("sha256")
        exp_bytes = entry.get("size_bytes")

        if not filename or not exp_sha or not isinstance(exp_bytes, int):
            all_valid = False
            binary_results.append({"filename": filename, "valid": False, "reason": "missing_required_entry_fields"})
            continue

        if filename in seen_filenames:
            all_valid = False
            binary_results.append({"filename": filename, "valid": False, "reason": "duplicate_filename"})
            continue
        seen_filenames.add(filename)

        target_path = os.path.abspath(os.path.join(bin_dir, filename))
        if not target_path.startswith(bin_dir + os.sep):
            all_valid = False
            binary_results.append({"filename": filename, "valid": False, "reason": "path_traversal_outside_bin"})
            continue

        if not os.path.isfile(target_path):
            all_valid = False
            binary_results.append({"filename": filename, "valid": False, "reason": "file_not_found"})
            continue

        actual_bytes = os.path.getsize(target_path)
        actual_sha = compute_sha256(target_path)

        matches = (actual_bytes == exp_bytes) and (actual_sha == exp_sha)
        if not matches:
            all_valid = False

        binary_results.append({
            "filename": filename,
            "valid": matches,
            "expected_bytes": exp_bytes,
            "actual_bytes": actual_bytes,
            "expected_sha256": exp_sha,
            "actual_sha256": actual_sha,
        })

    manifest_hash_after = compute_sha256(manifest_path)
    if manifest_hash_before != manifest_hash_after:
        all_valid = False

    return {
        "verified": all_valid,
        "manifest_path": manifest_path,
        "manifest_sha256_before": manifest_hash_before,
        "manifest_sha256_after": manifest_hash_after,
        "version": version,
        "git_commit": git_commit,
        "version_id": version_id,
        "binaries": binary_results,
    }


def evaluate_throughput_metric(
    metric_obj: Dict[str, Any],
    metric_name: str,
    norm_ref: str,
    impl_ref: str,
) -> Dict[str, Any]:
    iters = metric_obj.get("iterations")
    elapsed = metric_obj.get("elapsed_secs")
    ops = metric_obj.get("ops_per_sec")
    stats = metric_obj.get("latency_stats", {})
    count = stats.get("count")

    if (
        type(iters) is not int
        or iters <= 0
        or not isinstance(elapsed, (int, float))
        or isinstance(elapsed, bool)
        or not math.isfinite(elapsed)
        or elapsed <= 0.0
        or not isinstance(ops, (int, float))
        or isinstance(ops, bool)
        or not math.isfinite(ops)
        or ops <= 0.0
        or count != iters
        or not math.isclose(ops, iters / elapsed, rel_tol=0.001)
    ):
        return {
            "verdict": "failed",
            "metric": metric_name,
            "reason": "invalid_or_mismatched_iterations_samples_or_elapsed",
            "normative_ref": norm_ref,
            "impl_ref": impl_ref,
            "unit": "spans/s",
            "threshold": PARSER_THROUGHPUT_MIN_SPS,
            "mode": "strictly_greater",
            "iterations": iters,
            "elapsed": elapsed,
            "stats_count": count,
            "value": sanitize_for_json(ops),
        }

    return evaluate_metric_threshold(
        float(ops),
        PARSER_THROUGHPUT_MIN_SPS,
        "strictly_greater",
        metric_name,
        norm_ref,
        impl_ref,
        "spans/s",
    )


def evaluate_full_run(report: Dict[str, Any], candidate_hook_meta: Dict[str, Any]) -> Dict[str, Any]:
    verdicts = []

    # 1. Pure Parser Baseline
    p_pure = report.get("parser_pure_json_span", {})
    verdicts.append(evaluate_throughput_metric(
        p_pure,
        "parser_pure_json_span_throughput",
        NORMATIVE_REF_COORDINATION,
        "agent_otel_core::otlp::build_span_from_hook",
    ))

    # 2. Legacy Frame 0x01 Pipeline
    p01 = report.get("legacy_frame_0x01_pipeline", {})
    verdicts.append(evaluate_throughput_metric(
        p01,
        "legacy_frame_0x01_throughput",
        NORMATIVE_REF_COORDINATION,
        "agent_otel_ipc::frame::decode_header",
    ))

    # 3. Context Envelope 0x04 Pipeline
    p04 = report.get("envelope_0x04_pipeline", {})
    verdicts.append(evaluate_throughput_metric(
        p04,
        "envelope_0x04_throughput",
        NORMATIVE_REF_COORDINATION,
        "agent_otel_ipc::frame::decode_context_payload",
    ))

    # 4. Context Harvesting Fixtures
    ch_list = report.get("context_harvest", [])
    required_fixtures = {"git_repo", "git_worktree", "no_git"}
    seen_fixtures = set()
    for ch in ch_list:
        fixture = ch.get("fixture", "unknown")
        seen_fixtures.add(fixture)
        iters = ch.get("iterations", 0)
        stats = ch.get("stats", {})
        count = stats.get("count", 0)
        p99 = stats.get("p99_us")

        if not isinstance(iters, int) or iters <= 0 or count != iters or p99 is None:
            verdicts.append({
                "verdict": "failed",
                "metric": f"context_harvest_{fixture}_p99_us",
                "reason": "zero_iterations_or_mismatched_sample_count",
                "normative_ref": NORMATIVE_REF_COORDINATION,
                "impl_ref": "agent_otel_core::context::WorkspaceContext::harvest_from_dir",
                "unit": "microseconds",
                "threshold": HARVEST_LATENCY_MAX_US,
                "mode": "strictly_less",
                "value": sanitize_for_json(p99),
            })
        else:
            verdicts.append(evaluate_metric_threshold(
                float(p99),
                HARVEST_LATENCY_MAX_US,
                "strictly_less",
                f"context_harvest_{fixture}_p99_us",
                NORMATIVE_REF_COORDINATION,
                "agent_otel_core::context::WorkspaceContext::harvest_from_dir",
                "microseconds",
            ))

    for missing in required_fixtures - seen_fixtures:
        verdicts.append({
            "verdict": "failed",
            "metric": f"context_harvest_{missing}_p99_us",
            "reason": "required_fixture_missing",
            "normative_ref": NORMATIVE_REF_COORDINATION,
            "impl_ref": "agent_otel_core::context::WorkspaceContext::harvest_from_dir",
            "unit": "microseconds",
            "threshold": HARVEST_LATENCY_MAX_US,
            "mode": "strictly_less",
            "value": None,
        })

    # 5. Hook Binary Size (strictly on hook only, not daemon)
    if candidate_hook_meta.get("exists"):
        verdicts.append(candidate_hook_meta.get("size_evaluation", {
            "verdict": "failed",
            "metric": "hook_binary_size",
            "reason": "missing_size_evaluation",
        }))
    else:
        verdicts.append({
            "verdict": "failed",
            "metric": "hook_binary_size",
            "reason": "candidate_hook_binary_not_found",
            "normative_ref": NORMATIVE_REF_COORDINATION,
            "impl_ref": "crates/agent-otel-client",
            "unit": "bytes",
            "threshold": float(HOOK_BINARY_SIZE_MAX_BYTES),
            "mode": "strictly_less",
            "value": None,
        })

    # 6. Required Windows Observations (must NOT be proxy-passed)
    reported_not_measured = set(report.get("not_measured", []))
    for req in REQUIRED_WINDOWS_OBSERVATIONS:
        if req in reported_not_measured:
            verdicts.append({
                "verdict": "not_measured",
                "metric": req,
                "normative_ref": NORMATIVE_REF_COORDINATION,
                "impl_ref": "batch1_observation_boundary",
                "reason": "external_observation_unavailable_in_batch1_no_proxy_allowed",
                "outside_required_scope": False,
            })
        else:
            verdicts.append({
                "verdict": "failed",
                "metric": req,
                "reason": "missing_required_not_measured_declaration",
                "normative_ref": NORMATIVE_REF_COORDINATION,
                "impl_ref": "batch1_observation_boundary",
            })

    has_failures = any(v.get("verdict") == "failed" for v in verdicts)
    has_unmeasured_required = any(
        v.get("metric") in REQUIRED_WINDOWS_OBSERVATIONS and v.get("verdict") == "not_measured"
        for v in verdicts
    )

    if has_failures:
        overall_verdict = "failed"
    elif has_unmeasured_required:
        overall_verdict = "not_measured"
    else:
        overall_verdict = "passed"

    return {
        "fixture_conversation_id": report.get("fixture_conversation_id", "performance-fixture"),
        "overall_verdict": overall_verdict,
        "overall_passed": (overall_verdict == "passed"),
        "has_failures": has_failures,
        "has_unmeasured_required": has_unmeasured_required,
        "verdicts": verdicts,
    }


def run_driver(
    example_bin: str,
    hook_bin: str,
    debug_daemon_bin: Optional[str],
    release_daemon_bin: Optional[str],
    manifest_path: Optional[str],
    repeats: int = 3,
    timeout_secs: int = 60,
    build_profile: str = "release",
) -> int:
    if repeats < 1 or repeats > 5:
        sys.stderr.write(f"Error: repeats must be between 1 and 5, got {repeats}\n")
        return 1
    if timeout_secs < 1 or timeout_secs > 300:
        sys.stderr.write(f"Error: timeout must be between 1 and 300 seconds, got {timeout_secs}\n")
        return 1

    env_info = collect_environment()
    source_info = inspect_source_state()

    hook_meta = inspect_binary(hook_bin, is_hook_binary=True, build_profile=build_profile)
    example_meta = inspect_binary(example_bin, is_hook_binary=False, build_profile=build_profile)
    debug_daemon_meta = (
        inspect_binary(debug_daemon_bin, is_hook_binary=False, build_profile="debug")
        if debug_daemon_bin
        else None
    )
    release_daemon_meta = (
        inspect_binary(release_daemon_bin, is_hook_binary=False, build_profile="release")
        if release_daemon_bin
        else None
    )

    resolved_manifest = manifest_path if manifest_path else default_active_manifest_path()
    manifest_res = verify_active_manifest(resolved_manifest) if resolved_manifest else None

    runs = []
    aborted_early = False

    for r_idx in range(repeats):
        sys.stderr.write(f"Executing performance run {r_idx + 1}/{repeats}...\n")
        try:
            proc = subprocess.run(
                [example_bin],
                capture_output=True,
                text=True,
                timeout=timeout_secs,
                shell=False,
            )
            if proc.returncode != 0:
                sys.stderr.write(f"Run {r_idx + 1} failed with exit {proc.returncode}\n")
                runs.append({"run_index": r_idx, "exit_code": proc.returncode, "error": proc.stderr.strip()})
                aborted_early = True
                break

            report_json = json.loads(proc.stdout)
            eval_res = evaluate_full_run(report_json, hook_meta)
            runs.append({
                "run_index": r_idx,
                "exit_code": 0,
                "raw_report": report_json,
                "evaluation": eval_res,
            })
            if eval_res.get("overall_verdict") == "failed":
                sys.stderr.write(f"Run {r_idx + 1} produced failed SLA assertions; retaining planned measurement repetitions\n")

        except subprocess.TimeoutExpired as exc:
            sys.stderr.write(f"Run {r_idx + 1} timed out after {timeout_secs}s\n")
            runs.append({"run_index": r_idx, "error": "subprocess_timeout_expired", "timeout_seconds": timeout_secs})
            aborted_early = True
            break
        except OSError as exc:
            sys.stderr.write(f"Run {r_idx + 1} failed with OSError: {exc}\n")
            runs.append({"run_index": r_idx, "error": f"os_error: {exc}"})
            aborted_early = True
            break
        except Exception as exc:
            sys.stderr.write(f"Run {r_idx + 1} json decode error: {exc}\n")
            runs.append({"run_index": r_idx, "error": f"json_decode_error: {exc}"})
            aborted_early = True
            break

    all_runs_completed = len(runs) == repeats and not aborted_early
    verdicts = [r.get("evaluation", {}).get("overall_verdict", "failed") for r in runs]
    overall_verdict = ("failed" if not all_runs_completed or "failed" in verdicts else
                       "not_measured" if "not_measured" in verdicts else "passed")

    output_doc = {
        "environment": env_info,
        "source_provenance": source_info,
        "binaries": {
            "example_bin": example_meta,
            "candidate_hook": hook_meta,
            "debug_daemon": debug_daemon_meta,
            "release_daemon": release_daemon_meta,
        },
        "active_installed_manifest": manifest_res,
        "requested_repeats": repeats,
        "completed_repeats": len(runs),
        "aborted_early": aborted_early,
        "runs": runs,
        "overall_verdict": overall_verdict,
        "overall_passed": (overall_verdict == "passed"),
    }

    clean_doc = sanitize_for_json(output_doc)
    sys.stdout.write(json.dumps(clean_doc, indent=2))
    sys.stdout.write("\n")
    sys.stdout.flush()

    return 0 if overall_verdict == "passed" else 1


def main():
    parser = argparse.ArgumentParser(description="Bounded portable performance driver")
    exe_suffix = ".exe" if platform.system() == "Windows" else ""
    default_example = os.path.join("target", "release", "examples", f"performance{exe_suffix}")
    default_hook = os.path.join("target", "release", f"agent-hook{exe_suffix}")

    parser.add_argument("--example-bin", default=default_example, help="Path to compiled performance example")
    parser.add_argument("--hook-bin", default=default_hook, help="Path to candidate hook binary")
    parser.add_argument("--debug-daemon-bin", default=None, help="Path to debug daemon binary")
    parser.add_argument("--release-daemon-bin", default=None, help="Path to release daemon binary")
    parser.add_argument("--manifest", default=None, help="Path to active.json manifest (auto-discovers if omitted)")
    parser.add_argument("--repeats", type=int, default=3, help="Bounded benchmark repetitions (1-5)")
    parser.add_argument("--timeout", type=int, default=60, help="Subprocess timeout in seconds (1-300)")
    parser.add_argument("--build-profile", default="release", help="Build profile designation")

    args = parser.parse_args()
    exit_code = run_driver(
        args.example_bin,
        args.hook_bin,
        args.debug_daemon_bin,
        args.release_daemon_bin,
        args.manifest,
        args.repeats,
        args.timeout,
        args.build_profile,
    )
    sys.exit(exit_code)


if __name__ == "__main__":
    main()
