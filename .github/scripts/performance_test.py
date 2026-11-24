"""Tests for the performance gate rules."""

import copy
import io
import json
import unittest
from contextlib import redirect_stdout

from performance import main
from performance import (
    aggregate,
    read_json,
    compare,
    markdown,
    record_floor,
    stitch,
    validate_aggregate,
)

SHA = "0123456789abcdef0123456789abcdef01234567"


import tempfile
from pathlib import Path


def workload(name, budget, median):
    return {
        "name": name,
        "criterion": f"{name} workload",
        "budget_ms": budget,
        "samples": 9,
        "repeat": 1,
        "warmup": 3,
        "median_ms": median,
        "p95_ms": median + 0.1,
        "max_gap_ms": median + 0.2,
    }


def report(values):
    result = {
        "method": 1,
        "runtime": "rustc test",
        "os": "test",
        "cpu": "test",
        "workload_hash": "abc123",
        "results": [workload(*value) for value in values],
    }
    return aggregate([result, result, result], 3)


def floor_for(head):
    return record_floor(None, head, SHA, "2026-01-01T00:00:00Z")


class AggregateTest(unittest.TestCase):
    def test_keeps_best_run_and_its_spread(self):
        fast = report([("cli/decks", 100.0, 10.0)])
        slow = copy.deepcopy(fast)
        slow["results"][0]["median_ms"] = 14.0
        slow["results"][0]["p95_ms"] = 15.0
        slow["results"][0]["worst_p95_ms"] = 15.0
        slow["results"][0]["max_gap_ms"] = 16.0
        slow["results"][0]["worst_gap_ms"] = 16.0
        middle = copy.deepcopy(fast)
        middle["results"][0]["median_ms"] = 12.0
        combined = aggregate([fast, slow, middle], 3)
        self.assertEqual(combined["results"][0]["median_ms"], 10.0)
        self.assertEqual(combined["results"][0]["worst_median_ms"], 14.0)
        self.assertEqual(combined["results"][0]["p95_ms"], 10.1)
        self.assertEqual(combined["results"][0]["worst_p95_ms"], 15.0)
        self.assertEqual(combined["results"][0]["worst_gap_ms"], 16.0)

    def test_rejects_mixed_runners(self):
        first = report([("cli/decks", 100.0, 10.0)])
        second = copy.deepcopy(first)
        second["cpu"] = "another cpu"
        with self.assertRaises(Exception):
            aggregate([first, second, first], 3)


class GateTest(unittest.TestCase):
    def test_regression_beyond_margin_fails(self):
        ceiling = report([("mcp/roundtrip", 100.0, 10.0)])
        head = report([("mcp/roundtrip", 100.0, 11.0)])
        result = compare(head, ceiling=ceiling, floor=floor_for(ceiling))
        self.assertTrue(result["failed"])
        self.assertIn("ceiling", markdown(result))

    def test_small_noise_passes(self):
        ceiling = report([("core/fsrs", 100.0, 10.0)])
        head = report([("core/fsrs", 100.0, 10.1)])
        self.assertFalse(compare(head, ceiling=ceiling, floor=floor_for(ceiling))["failed"])

    def test_raised_budget_fails(self):
        ceiling = report([("storage/search", 100.0, 10.0)])
        head = report([("storage/search", 200.0, 9.0)])
        result = compare(head, ceiling=ceiling, floor=floor_for(ceiling))
        self.assertTrue(result["failed"])
        self.assertEqual(result["rows"][0]["findings"][0]["kind"], "loosened")

    def test_missing_workload_fails(self):
        ceiling = report([("storage/search", 100.0, 10.0)])
        head = report([("core/fsrs", 100.0, 10.0)])
        result = compare(head, ceiling=ceiling, floor=floor_for(ceiling))
        self.assertTrue(result["failed"])
        self.assertTrue(any(row["status"] == "fail" and row["name"] == "storage/search" for row in result["rows"]))

    def test_method_change_fails_against_recorded_ceiling(self):
        ceiling = report([("gui/frame/idle", 100.0, 10.0)])
        head = copy.deepcopy(report([("gui/frame/idle", 100.0, 30.0)]))
        head["method"] = 2
        result = compare(head, ceiling=ceiling, floor=floor_for(ceiling))
        self.assertTrue(result["failed"])
        self.assertTrue(any(item["kind"] == "method" for item in result["global_findings"]))

    def test_reduced_sampling_fails(self):
        ceiling = report([("gui/frame/idle", 100.0, 10.0)])
        head = copy.deepcopy(ceiling)
        head["results"][0]["samples"] = 1
        head["results"][0]["repeat"] = 1
        head["results"][0]["warmup"] = 0
        result = compare(head, ceiling=ceiling, floor=floor_for(ceiling))
        self.assertTrue(result["failed"])
        self.assertTrue(any(item["kind"] == "sampling" for item in result["rows"][0]["findings"]))

    def test_reduced_sampling_fails_even_when_ceiling_is_remade_with_it(self):
        ceiling = report([("gui/frame/idle", 100.0, 10.0)])
        floor = floor_for(ceiling)
        head = copy.deepcopy(ceiling)
        head["results"][0]["samples"] = 1
        head["results"][0]["repeat"] = 1
        head["results"][0]["warmup"] = 0
        result = compare(head, ceiling=head, floor=floor)
        self.assertTrue(result["failed"])
        self.assertTrue(any(item["kind"] == "sampling" for item in result["rows"][0]["findings"]))

    def test_ceiling_directory_uses_recorded_commit(self):
        ceiling = report([("gui/frame/idle", 100.0, 10.0)])
        floor = floor_for(ceiling)
        head = copy.deepcopy(ceiling)
        for key in ("median_ms", "worst_median_ms", "p95_ms", "worst_p95_ms"):
            head["results"][0][key] *= 3.0
        with tempfile.TemporaryDirectory() as directory:
            (Path(directory) / f"{SHA}.json").write_text(
                json.dumps(ceiling), encoding="utf-8"
            )
            result = compare(head, ceiling_dir=directory, floor=floor)
            self.assertTrue(result["failed"])
            self.assertFalse(
                any(item["kind"] == "ceiling" for item in result["global_findings"])
            )
            result = compare(head, ceiling_dir=directory, floor=floor_for(head))
            self.assertTrue(result["failed"])

    def test_missing_ceiling_report_fails(self):
        ceiling = report([("gui/frame/idle", 100.0, 10.0)])
        with tempfile.TemporaryDirectory() as directory:
            result = compare(ceiling, ceiling_dir=directory, floor=floor_for(ceiling))
            self.assertTrue(result["failed"])
            self.assertTrue(
                any(item["kind"] == "ceiling" for item in result["global_findings"])
            )

    def test_changed_workload_definition_fails(self):
        ceiling = report([("gui/frame/idle", 100.0, 10.0)])
        head = copy.deepcopy(ceiling)
        head["workload_hash"] = "deadbeef"
        result = compare(head, ceiling=ceiling, floor=floor_for(ceiling))
        self.assertTrue(result["failed"])
        self.assertTrue(any(item["kind"] == "workload" for item in result["global_findings"]))

    def test_empty_floor_without_base_fails(self):
        head = report([("process/cli/decks", 100.0, 10.0)])
        result = compare(head)
        self.assertTrue(result["failed"])
        self.assertTrue(
            any(item["kind"] == "reference" for item in result["global_findings"])
        )

    def test_no_ceiling_uses_base_as_reference(self):
        base = report([("process/cli/decks", 100.0, 10.0)])
        head = report([("process/cli/decks", 100.0, 11.0)])
        result = compare(head, base=base)
        self.assertTrue(result["failed"])
        self.assertIn("no ceiling", markdown(result))


class FloorTest(unittest.TestCase):
    def test_plan_schedules_each_metric_sha(self):
        ceiling = report([("core/fsrs", 100.0, 10.0)])
        floor = floor_for(ceiling)
        other = "abcdef0123456789abcdef0123456789abcdef01"
        floor["metrics"]["core/fsrs"]["sha"] = other
        with tempfile.TemporaryDirectory() as directory:
            floor_path = Path(directory) / "floor.json"
            meta_path = Path(directory) / "meta.json"
            floor_path.write_text(json.dumps(floor), encoding="utf-8")
            output = io.StringIO()
            with redirect_stdout(output):
                main([
                    "plan",
                    "--head", SHA,
                    "--base", "1111111111111111111111111111111111111111",
                    "--floor", str(floor_path),
                    "--kind", "pull_request",
                    "--out", str(meta_path),
                ])
            self.assertIn(other, output.getvalue().split())
            meta = json.loads(meta_path.read_text(encoding="utf-8"))
            self.assertEqual(meta["floor"]["core/fsrs"], other)

    def test_ceiling_only_lowers_when_worst_run_beats_it(self):
        first = report([("core/sm2", 100.0, 10.0)])
        floor = floor_for(first)
        noisy = report([("core/sm2", 100.0, 9.0)])
        noisy["results"][0]["worst_median_ms"] = 11.0
        kept = record_floor(floor, noisy, SHA, "2026-01-02T00:00:00Z")
        self.assertEqual(kept["metrics"]["core/sm2"]["median_ms"], 10.0)
        improved = report([("core/sm2", 50.0, 8.0)])
        improved["results"][0]["worst_median_ms"] = 8.5
        lowered = record_floor(kept, improved, SHA, "2026-01-03T00:00:00Z")
        self.assertEqual(lowered["metrics"]["core/sm2"]["median_ms"], 8.0)
        self.assertEqual(lowered["metrics"]["core/sm2"]["budget_ms"], 50.0)
        validate_aggregate(improved)

class StitchTest(unittest.TestCase):
    def test_each_workload_uses_the_shard_that_measured_its_ceiling(self):
        first = "1111111111111111111111111111111111111111"
        second = "2222222222222222222222222222222222222222"
        head_a = report([("core/fsrs", 100.0, 10.0), ("storage/search", 100.0, 90.0)])
        head_b = report([("core/fsrs", 100.0, 90.0), ("storage/search", 100.0, 20.0)])
        ceiling_a = report([("core/fsrs", 100.0, 12.0), ("storage/search", 100.0, 95.0)])
        ceiling_b = report([("core/fsrs", 100.0, 95.0), ("storage/search", 100.0, 22.0)])
        floor = floor_for(head_a)
        floor["metrics"]["core/fsrs"]["sha"] = first
        floor["metrics"]["storage/search"]["sha"] = second
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            shards = root / "shards"
            out = root / "out"
            floor_path = root / "floor.json"
            floor_path.write_text(json.dumps(floor), encoding="utf-8")
            for shard, values in ((first, (head_a, ceiling_a)), (second, (head_b, ceiling_b))):
                target = shards / shard
                target.mkdir(parents=True)
                for round_number in range(1, 4):
                    for sha, value in ((SHA, values[0]), (shard, values[1])):
                        (target / f"{sha}-{round_number}.json").write_text(
                            json.dumps(value), encoding="utf-8"
                        )
            stitch(str(shards), str(floor_path), SHA, str(out))
            head = validate_aggregate(read_json(out / f"{SHA}.json"))
            results = {item["name"]: item for item in head["results"]}
            self.assertEqual(results["core/fsrs"]["median_ms"], 10.0)
            self.assertEqual(results["storage/search"]["median_ms"], 20.0)
            first_ceiling = validate_aggregate(read_json(out / "ceilings" / f"{first}.json"))
            second_ceiling = validate_aggregate(read_json(out / "ceilings" / f"{second}.json"))
            self.assertEqual(first_ceiling["results"][0]["median_ms"], 12.0)
            self.assertEqual(second_ceiling["results"][1]["median_ms"], 22.0)


    def test_empty_floor_uses_head_from_the_only_shard(self):
        base = "1111111111111111111111111111111111111111"
        head_value = report([("core/fsrs", 100.0, 10.0)])
        base_value = report([("core/fsrs", 100.0, 12.0)])
        floor = floor_for(head_value)
        floor["metrics"] = {}
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target = root / "shards" / base
            target.mkdir(parents=True)
            floor_path = root / "floor.json"
            floor_path.write_text(json.dumps(floor), encoding="utf-8")
            for round_number in range(1, 4):
                (target / f"{SHA}-{round_number}.json").write_text(
                    json.dumps(head_value), encoding="utf-8"
                )
                (target / f"{base}-{round_number}.json").write_text(
                    json.dumps(base_value), encoding="utf-8"
                )
            stitch(str(root / "shards"), str(floor_path), SHA, str(root / "out"))
            head = validate_aggregate(read_json(root / "out" / f"{SHA}.json"))
            self.assertEqual(head["results"][0]["median_ms"], 10.0)
            self.assertTrue((root / "out" / f"{base}.json").is_file())
            self.assertFalse((root / "out" / "ceilings" / f"{SHA}.json").exists())

    def test_floor_sha_equal_to_head_still_writes_a_ceiling(self):
        base = "1111111111111111111111111111111111111111"
        head_value = report([("core/fsrs", 100.0, 10.0)])
        base_value = report([("core/fsrs", 100.0, 12.0)])
        floor = floor_for(head_value)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            target = root / "shards" / base
            target.mkdir(parents=True)
            floor_path = root / "floor.json"
            floor_path.write_text(json.dumps(floor), encoding="utf-8")
            for round_number in range(1, 4):
                (target / f"{SHA}-{round_number}.json").write_text(
                    json.dumps(head_value), encoding="utf-8"
                )
                (target / f"{base}-{round_number}.json").write_text(
                    json.dumps(base_value), encoding="utf-8"
                )
            out = root / "out"
            stitch(str(root / "shards"), str(floor_path), SHA, str(out))
            ceiling = out / "ceilings" / f"{SHA}.json"
            self.assertTrue(ceiling.is_file())
            head = validate_aggregate(read_json(out / f"{SHA}.json"))
            result = compare(head, ceiling_dir=out / "ceilings", floor=floor)
            self.assertFalse(result["failed"])
            self.assertFalse(
                any(item["kind"] == "ceiling" for item in result["global_findings"])
            )



if __name__ == "__main__":
    unittest.main()
