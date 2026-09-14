import importlib.util
import unittest
from pathlib import Path

spec=importlib.util.spec_from_file_location("production_round",Path(__file__).parents[1]/"production_round.py")
m=importlib.util.module_from_spec(spec); spec.loader.exec_module(m)

class ProductionRoundTests(unittest.TestCase):
    def valid_benchmark(self):
        latency = {"count": m.LOOKUP_SAMPLES, "p50_us": 1.0, "p95_us": 2.0,
                   "p99_us": 2.0, "max_us": 2.0}
        return {"schema": m.BENCH_SCHEMA, "profile": "release", "seed": m.BENCH_SEED,
                "warmup_iterations": 2000, "corpus_items": 4,
                "corpus_identity": "deterministic_mixed_v1_0x01_0x04_tokens_ws",
                "semantic_assert_passed": True, "gate_parser_gt_50k_passed": True,
                "parser_only": {"iterations": m.BENCH_ITERATIONS, "elapsed_secs": 0.5,
                                "ops_per_sec": 100000.0},
                "production_transform": {"iterations": m.BENCH_ITERATIONS, "elapsed_secs": 1.0,
                                         "ops_per_sec": 50000.0,
                                         "boundary": "decode_frame + parse_slice + context_cache_lookup + normalize + build_span_from_resolved; excludes quota, clock, batch, export"},
                "lookup_hit": latency,
                "lookup_miss": {"distribution": latency, "queued_count": 5000,
                                "queue_full_count": 5000},
                "lookup_stale": {"distribution": latency, "queued_count": 1,
                                 "already_pending_count": m.LOOKUP_SAMPLES - 1},
                "boundary_notes": "exact measured and excluded boundaries"}

    def valid_ipc_benchmark(self):
        stats = {"count": m.IPC_TIMED_ROUNDS, "min_us": 1.0, "mean_us": 2.0,
                 "p50_us": 2.0, "p95_us": 3.0, "p99_us": 4.0, "max_us": 5.0}
        result = lambda msg_type: {
            "msg_type": msg_type, "expected_rounds": m.IPC_TIMED_ROUNDS,
            "warmup_rounds": m.IPC_WARMUP_ROUNDS, "success_count": m.IPC_TIMED_ROUNDS,
            "failure_count": 0, "latency_stats": dict(stats), "p99_sla_passed": True}
        return {"conversation_id": "performance-fixture", "pipe_name": "owned-endpoint",
                "latency_label": "client_send_through_receipt_one_way_observer_micros",
                "normative_ref": "required measurements", "impl_ref": "client to server",
                "results": [result("HookPayload"), result("HookPayloadWithContext")],
                "overall_passed": True,
                "not_measured": ["hook_internal_execution_duration_us"]}

    def test_real_schema_and_diagnostic_transform(self):
        report = self.valid_benchmark()
        self.assertTrue(m.validate_benchmark(report))
        report["parser_only"] = {"iterations": m.BENCH_ITERATIONS,
                                 "elapsed_secs": 1.0, "ops_per_sec": 50000.0}
        self.assertFalse(m.validate_benchmark(report))

    def test_rejects_false_semantics_and_bad_disposition_counts(self):
        report = self.valid_benchmark()
        report["semantic_assert_passed"] = False
        self.assertFalse(m.validate_benchmark(report))
        report = self.valid_benchmark()
        report["lookup_miss"]["queue_full_count"] = 0
        self.assertFalse(m.validate_benchmark(report))

    def test_nonfinite_bool_and_inconsistent_rate(self):
        for value in (float("nan"), float("inf"), True, 0, -1):
            report = self.valid_benchmark()
            report["parser_only"]["elapsed_secs"] = value
            self.assertFalse(m.validate_benchmark(report))
        report = self.valid_benchmark()
        report["parser_only"]["ops_per_sec"] = 70000
        self.assertFalse(m.validate_benchmark(report))

    def test_benchmark_requires_provenance_and_fixed_corpus_fields(self):
        for field in ("profile", "seed", "warmup_iterations", "corpus_items",
                      "corpus_identity", "boundary_notes", "gate_parser_gt_50k_passed"):
            report = self.valid_benchmark()
            del report[field]
            self.assertFalse(m.validate_benchmark(report), field)

    def test_ipc_benchmark_requires_exact_counts_types_and_p99(self):
        report = self.valid_ipc_benchmark()
        self.assertTrue(m.validate_ipc_benchmark(report))
        report["results"][0]["failure_count"] = 1
        self.assertFalse(m.validate_ipc_benchmark(report))
        report = self.valid_ipc_benchmark()
        report["results"][1]["latency_stats"]["p99_us"] = m.IPC_P99_LIMIT_US
        self.assertFalse(m.validate_ipc_benchmark(report))
        report = self.valid_ipc_benchmark()
        report["results"][1]["msg_type"] = "HookPayload"
        self.assertFalse(m.validate_ipc_benchmark(report))

    def test_provenance_separates_assignment_from_actual_execution(self):
        self.assertEqual(m.MODEL_ROLES["author_initial_assignment"],
                         "agy/gemini-3.8-flash-low")
        self.assertEqual(m.MODEL_ROLES["author_actual_execution"],
                         "agy/gemini-3.8-flash-medium relay escalation")

    def test_missing_hashes_and_empty_architecture_do_not_pass(self):
        self.assertFalse(m.snapshot_valid({"binaries": {"hook": None}}))
        self.assertFalse(m.validate_architecture({"schema": "agent-otel-new-architecture/v1", "overall_verdict": "passed", "scenarios": []}))

    def test_benchmark_gate(self):
        self.assertFalse(m.validate_benchmark({}))
        self.assertFalse(m.validate_benchmark({"parser_only":{"iterations":100,"elapsed_secs":1,"ops_per_sec":100}}))
        self.assertFalse(m.validate_benchmark({"parser_only":{"iterations":1,"elapsed_secs":1,"ops_per_sec":float("nan")}}))

    def test_report_schema_and_required_probe(self):
        self.assertFalse(m.validate_report({"schema":"old","attempt_id":"x","scenarios":[]}))
        self.assertFalse(m.validate_report({"schema":m.SCHEMA,"attempt_id":"x","scenarios":[{"verdict":"passed","required_probes":["not_measured"]}]}))

    def test_ledger_reuse_and_limit(self):
        import tempfile
        with tempfile.TemporaryDirectory() as d:
            m.reserve(Path(d),1)
            with self.assertRaises(ValueError): m.reserve(Path(d),1)
            with self.assertRaises(ValueError): m.reserve(Path(d),4)
