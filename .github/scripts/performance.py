#!/usr/bin/env python3
"""Aggregate, gate, and record Ranki performance measurements.

The benchmark binary only emits one report per run. This script owns the rules
that decide whether a pull request is allowed to merge, so the audit workflow
always loads it from the default branch rather than from the measured commit.
"""

import argparse
import json
import re
import sys
from datetime import datetime, timezone
from pathlib import Path

VERSION = 1
FAIL_MARGIN = 0.05
NAME = re.compile(r"^[a-z0-9][a-z0-9/_-]*$")
SHA = re.compile(r"^[0-9a-f]{40}$")
COMMENT_MARKER = "<!-- ranki-performance -->"


class Invalid(Exception):
    pass


def fail(message):
    raise Invalid(message)


def finite(value, label, minimum=0.0):
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        fail(f"{label} must be a number")
    if not -float("inf") < value < float("inf") or value < minimum:
        fail(f"{label} must be finite and >= {minimum}")
    return float(value)


def read_json(path):
    return json.loads(Path(path).read_text(encoding="utf-8"))


def write_json(path, value):
    Path(path).write_text(json.dumps(value, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def validate_report(report, label="report"):
    if not isinstance(report, dict):
        fail(f"{label} must be an object")
    for key in ("runtime", "os", "cpu", "workload_hash"):
        if not isinstance(report.get(key), str) or not report[key]:
            fail(f"{label}.{key} must be a string")
    if not isinstance(report.get("method"), int) or report["method"] < 1:
        fail(f"{label}.method must be a positive integer")
    results = report.get("results")
    if not isinstance(results, list) or not results:
        fail(f"{label}.results must be a non-empty array")
    names = set()
    for index, result in enumerate(results):
        prefix = f"{label}.results[{index}]"
        name = result.get("name")
        if not isinstance(name, str) or not NAME.match(name):
            fail(f"{prefix}.name is invalid")
        if name in names:
            fail(f"{label} has a duplicate metric {name}")
        names.add(name)
        if not isinstance(result.get("criterion"), str) or not result["criterion"]:
            fail(f"{prefix}.criterion must be a string")
        finite(result.get("budget_ms"), f"{prefix}.budget_ms", 1e-12)
        for key in ("samples", "repeat", "warmup"):
            value = result.get(key)
            if not isinstance(value, int) or isinstance(value, bool) or value < 0:
                fail(f"{prefix}.{key} must be a non-negative integer")
        if result["samples"] < 1 or result["repeat"] < 1:
            fail(f"{prefix}.samples and .repeat must be positive")
        finite(result.get("median_ms"), f"{prefix}.median_ms", 1e-12)
        finite(result.get("p95_ms"), f"{prefix}.p95_ms", 1e-12)
        finite(result.get("max_gap_ms"), f"{prefix}.max_gap_ms")
    return report


def validate_aggregate(value, label="aggregate"):
    validate_report(value, label)
    for result in value["results"]:
        prefix = f"{label}.{result['name']}"
        finite(result.get("worst_median_ms"), f"{prefix}.worst_median_ms", 1e-12)
        finite(result.get("worst_p95_ms"), f"{prefix}.worst_p95_ms", 1e-12)
        finite(result.get("worst_gap_ms"), f"{prefix}.worst_gap_ms")
        if result["worst_median_ms"] < result["median_ms"]:
            fail(f"{prefix}.worst_median_ms is below its median_ms")
    return value


def validate_floor(value, label="floor"):
    if not isinstance(value, dict):
        fail(f"{label} must be an object")
    if value.get("version") != VERSION:
        fail(f"{label}.version must be {VERSION}")
    if not isinstance(value.get("workload_hash"), str):
        fail(f"{label}.workload_hash must be a string")
    if not isinstance(value.get("metrics"), dict):
        fail(f"{label}.metrics must be an object")
    budgets = value.get("budgets", {})
    if not isinstance(budgets, dict):
        fail(f"{label}.budgets must be an object")
    for name, budget in budgets.items():
        if not NAME.match(name):
            fail(f"{label}.budgets has an invalid name")
        finite(budget, f"{label}.budgets.{name}", 1e-12)
    for name, record in value["metrics"].items():
        if not NAME.match(name):
            fail(f"{label}.metrics has an invalid name")
        if not SHA.match(str(record.get("sha", ""))):
            fail(f"{label}.metrics.{name}.sha must be a full commit sha")
        finite(record.get("median_ms"), f"{label}.metrics.{name}.median_ms", 1e-12)
        finite(record.get("budget_ms"), f"{label}.metrics.{name}.budget_ms", 1e-12)
        for key in ("samples", "repeat", "warmup"):
            field = record.get(key)
            if not isinstance(field, int) or isinstance(field, bool) or field < 0:
                fail(f"{label}.metrics.{name}.{key} must be a non-negative integer")
    return value


def empty_floor():
    return {"version": VERSION, "sha": "", "method": 0, "workload_hash": "", "budgets": {}, "metrics": {}}


def read_floor(path):
    if not path:
        return empty_floor()
    if not Path(path).is_file():
        fail(f"floor file not found: {path}")
    return validate_floor(read_json(path), path)


def aggregate(reports, rounds):
    if not reports or len(reports) != rounds:
        fail(f"expected {rounds} runs, got {len(reports)}")
    for report in reports:
        validate_report(report)
    first = reports[0]
    names = [result["name"] for result in first["results"]]
    for report in reports[1:]:
        for field in ("method", "runtime", "os", "cpu", "workload_hash"):
            if report[field] != first[field]:
                fail(f"runs disagree about {field}")
        if [result["name"] for result in report["results"]] != names:
            fail("runs do not share the same metric set")
    results = []
    for name in names:
        values = []
        for report in reports:
            result = next(item for item in report["results"] if item["name"] == name)
            values.append(result)
        definition = ("criterion", "budget_ms", "samples", "repeat", "warmup")
        if any(tuple(item[key] for key in definition) != tuple(values[0][key] for key in definition) for item in values):
            fail(f"runs disagree about {name}")
        medians = [item["median_ms"] for item in values]
        p95s = [item["p95_ms"] for item in values]
        gaps = [item["max_gap_ms"] for item in values]
        best = min(medians)
        worst = max(medians)
        results.append({
            "name": name,
            "criterion": values[0]["criterion"],
            "budget_ms": values[0]["budget_ms"],
            "samples": values[0]["samples"],
            "repeat": values[0]["repeat"],
            "warmup": values[0]["warmup"],
            "median_ms": best,
            "worst_median_ms": worst,
            "p95_ms": min(p95s),
            "worst_p95_ms": max(p95s),
            "max_gap_ms": min(gaps),
            "worst_gap_ms": max(gaps),
            "noise_percent": (worst / best - 1.0) * 100.0,
        })
    combined = {
        key: first[key]
        for key in ("method", "runtime", "os", "cpu", "workload_hash")
    }
    combined["results"] = results
    return validate_aggregate(combined)


def result_map(report):
    return {item["name"]: item for item in report["results"]} if report else {}


def regression(label, value, best, worst, own_best, own_worst):
    bound = worst if worst > 0 else best
    margin = max(bound * FAIL_MARGIN, worst - best, own_worst - own_best)
    if value > bound + margin:
        return {
            "kind": "ceiling",
            "failed": True,
            "note": f"{label} {precise(value)}ms is more than {precise(margin)}ms over the ceiling's worst run",
        }
    if value > bound + margin / 2.0:
        return {
            "kind": "ceiling",
            "failed": False,
            "note": f"{label} {precise(value)}ms is above the ceiling's worst run",
        }
    return None


def compare(head, base=None, ceiling=None, floor=None, ceiling_dir=None):
    validate_aggregate(head)
    floor = floor or empty_floor()
    after = result_map(head)
    main = result_map(base)
    reference = {
        result["name"]: (result, ceiling["method"])
        for result in ceiling["results"]
    } if ceiling else {}
    if ceiling_dir:
        for name, record in floor.get("metrics", {}).items():
            report_path = Path(ceiling_dir) / f"{record['sha']}.json"
            if not report_path.is_file():
                continue
            report = validate_aggregate(read_json(report_path), str(report_path))
            result = result_map(report).get(name)
            if result:
                reference[name] = (result, report["method"])
    global_findings = []
    if floor.get("method") and head["method"] != floor["method"]:
        global_findings.append({
            "kind": "method",
            "failed": True,
            "note": f"timing method changed from {floor['method']} to {head['method']} against the recorded ceiling",
        })
    if floor.get("workload_hash") and head["workload_hash"] != floor["workload_hash"]:
        global_findings.append({
            "kind": "workload",
            "failed": True,
            "note": "benchmark workload definition changed since the recorded ceiling",
        })
    if ceiling_dir:
        missing = [
            name
            for name, record in floor.get("metrics", {}).items()
            if name not in reference
        ]
        if missing:
            global_findings.append({
                "kind": "ceiling",
                "failed": True,
                "note": f"missing re-measured ceiling for {', '.join(missing[:3])}",
            })
    budgets = dict(floor.get("budgets", {}))
    for name, record in floor.get("metrics", {}).items():
        budgets[name] = min(budgets.get(name, record["budget_ms"]), record["budget_ms"])
    names = sorted(set(after) | set(main) | set(floor.get("metrics", {})))
    rows = []
    for name in names:
        current = after.get(name)
        ceiling_entry = reference.get(name)
        ceiling_result = ceiling_entry[0] if ceiling_entry else None
        compared_with_base = ceiling_result is None and name in main
        if compared_with_base:
            ceiling_result = main[name]
        trusted = budgets.get(name)
        findings = []
        if current and ceiling_result:
            sampling = ("samples", "repeat", "warmup")
            recorded = floor.get("metrics", {}).get(name) or ceiling_result
            if any(current.get(key) != recorded.get(key) for key in sampling):
                findings.append({
                    "kind": "sampling",
                    "failed": True,
                    "note": "sampling definition differs from the recorded ceiling",
                })
        if trusted is not None and current and current["budget_ms"] > trusted:
            findings.append({
                "kind": "loosened",
                "failed": True,
                "note": f"budget raised from {precise(trusted)}ms to {precise(current['budget_ms'])}ms",
            })
        if current is None:
            findings.append({
                "kind": "missing",
                "failed": True,
                "note": "workload is not measured by this commit",
            })
        else:
            if not current.get("criterion"):
                findings.append({"kind": "criterion", "failed": True, "note": "no criterion declared"})
            budget = min(trusted, current["budget_ms"]) if trusted is not None else current["budget_ms"]
            if budget <= 0:
                findings.append({"kind": "budget", "failed": True, "note": "no budget declared"})
            elif current["median_ms"] > budget * (1.0 + FAIL_MARGIN):
                judgeable = ceiling_result is not None and ceiling_result["median_ms"] <= budget
                findings.append({
                    "kind": "budget",
                    "failed": judgeable,
                    "note": (
                        f"median {precise(current['median_ms'])}ms exceeds the {precise(budget)}ms budget"
                        if judgeable
                        else f"median {precise(current['median_ms'])}ms exceeds the {precise(budget)}ms budget, and so does the ceiling"
                    ),
                })
            if compared_with_base:
                findings.append({
                    "kind": "baseline",
                    "failed": False,
                    "note": "no ceiling yet, compared with base",
                })
            reference_method = ceiling_entry[1] if ceiling_entry else (base["method"] if compared_with_base and base else None)
            if ceiling_result and reference_method == head["method"]:
                for finding in [
                    regression("median", current["median_ms"], ceiling_result["median_ms"], ceiling_result["worst_median_ms"], current["median_ms"], current["worst_median_ms"]),
                    regression("p95", current["p95_ms"], ceiling_result["p95_ms"], ceiling_result["worst_p95_ms"], current["p95_ms"], current["worst_p95_ms"]),
                    regression("worst gap", current["max_gap_ms"], ceiling_result["max_gap_ms"], ceiling_result["worst_gap_ms"], current["max_gap_ms"], current["worst_gap_ms"]),
                ]:
                    if finding:
                        findings.append(finding)
            else:
                findings.append({
                    "kind": "baseline",
                    "failed": False,
                    "note": "timing method changed or no ceiling yet, so only budgets apply",
                })
        failed = any(item["failed"] for item in findings)
        warned = any(not item["failed"] for item in findings)
        status = "fail" if failed else "warn" if warned else "ok" if ceiling_result else "new"
        rows.append({
            "name": name,
            "criterion": current["criterion"] if current else "",
            "budget_ms": min(trusted, current["budget_ms"]) if trusted is not None and current else (trusted if trusted is not None else (current["budget_ms"] if current else None)),
            "ceiling_ms": ceiling_result["median_ms"] if ceiling_result else None,
            "after_ms": current["median_ms"] if current else None,
            "after_p95_ms": current["p95_ms"] if current else None,
            "after_gap_ms": current["max_gap_ms"] if current else None,
            "noise_percent": current["noise_percent"] if current else None,
            "change": (current["median_ms"] / ceiling_result["median_ms"] - 1.0) * 100.0 if current and ceiling_result else None,
            "findings": findings,
            "status": status,
        })
    return {
        "rows": rows,
        "global_findings": global_findings,
        "failed": any(item["failed"] for item in global_findings)
        or any(row["status"] == "fail" for row in rows),
    }


def precise(value):
    if value is None:
        return "n/a"
    if value < 0.01:
        return f"{value:.6f}"
    return f"{value:.3f}" if value < 10 else f"{value:.1f}"


def milliseconds(value):
    return "n/a" if value is None else f"{precise(value)}ms"


def signed(value):
    return "n/a" if value is None else f"{value:+.1f}%"


def markdown(comparison):
    rows = comparison["rows"]
    global_findings = comparison.get("global_findings", [])
    failures = sum(row["status"] == "fail" for row in rows)
    warnings = sum(row["status"] == "warn" for row in rows)
    if any(item["failed"] for item in global_findings) and failures == 0:
        summary = "❌ The recorded workload definition no longer matches this commit."
    elif failures:
        summary = f"❌ {failures} of {len(rows)} workloads failed the performance gate."
    elif warnings:
        summary = f"⚠️ {warnings} of {len(rows)} workloads are close to a ceiling or budget."
    else:
        summary = f"✅ All {len(rows)} workloads are within their ceilings and budgets."
    notes = [f"- {item['note']}" for item in global_findings]
    output = [
        COMMENT_MARKER,
        "### Performance audit",
        "",
        summary,
    ]
    if notes:
        output.extend(["", *notes])
    output.extend([
        "",
        "| Workload | Budget | Ceiling | This PR | Tail (p95 / gap) | vs ceiling | Status |",
        "| :------- | -----: | ------: | ------: | ---------------: | ---------: | :----- |",
    ])
    icons = {"fail": "❌", "warn": "⚠️", "new": "🆕", "ok": "✅"}
    for row in rows:
        criterion = row["criterion"].replace("\\", "\\\\").replace("|", "\\|").replace("\n", " ")
        criterion = f"<br><sub>{criterion}</sub>" if criterion else ""
        notes = "; ".join(item["note"] for item in row["findings"])
        icon = icons[row["status"]]
        status = f"{icon} {notes}" if notes else icon
        tail = f"{precise(row['after_p95_ms'])} / {precise(row['after_gap_ms'])}" if row["after_ms"] is not None else "n/a"
        output.append(
            f"| `{row['name']}`{criterion} | {milliseconds(row['budget_ms'])} | "
            f"{milliseconds(row['ceiling_ms'])} | {milliseconds(row['after_ms'])} | {tail} | "
            f"{signed(row['change'])} | {status} |"
        )
    output.extend([
        "",
        "Every workload is the best of three runs with warmup and repeated samples. "
        "A workload fails when its median, p95, or worst operation gap exceeds the ceiling's "
        f"worst run by more than {FAIL_MARGIN * 100:.0f}% or either side's spread between rounds. "
        "Budgets can only be tightened. Ceilings come from the `floors` branch and are "
        "re-measured in the same session.",
        "",
    ])
    return "\n".join(output)


def record_floor(previous, head, sha, now):
    validate_aggregate(head)
    previous = previous or empty_floor()
    if not SHA.match(sha):
        fail("--sha must be a full commit sha")
    budgets = dict(previous.get("budgets", {}))
    metrics = {}
    for result in head["results"]:
        name = result["name"]
        old = previous.get("metrics", {}).get(name)
        budget = min(value for value in [budgets.get(name), old["budget_ms"] if old else None, result["budget_ms"]] if value is not None)
        budgets[name] = budget
        keep_old = bool(
            old
            and old.get("method") == head["method"]
            and all(old.get(key) == result[key] for key in ("samples", "repeat", "warmup"))
            and result["worst_median_ms"] >= old["median_ms"]
        )
        if keep_old:
            metrics[name] = {**old, "budget_ms": budget}
        else:
            metrics[name] = {
                "sha": sha,
                "method": head["method"],
                "samples": result["samples"],
                "repeat": result["repeat"],
                "warmup": result["warmup"],
                "median_ms": result["median_ms"],
                "worst_median_ms": result["worst_median_ms"],
                "budget_ms": budget,
                "updated_at": now,
            }
    return validate_floor({
        "version": VERSION,
        "sha": sha,
        "method": head["method"],
        "workload_hash": head["workload_hash"],
        "budgets": budgets,
        "metrics": metrics,
    })


def parse_args(argv):
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    bootstrap = commands.add_parser("bootstrap")
    bootstrap.add_argument("--out", required=True)
    plan = commands.add_parser("plan")
    plan.add_argument("--head", required=True)
    plan.add_argument("--base")
    plan.add_argument("--floor")
    plan.add_argument("--kind", choices=("pull_request", "push"), required=True)
    plan.add_argument("--out", required=True)
    group = commands.add_parser("aggregate")
    group.add_argument("--dir", required=True)
    group.add_argument("--out", required=True)
    group.add_argument("--rounds", type=int, default=3)
    compare_command = commands.add_parser("compare")
    compare_command.add_argument("--head", required=True)
    compare_command.add_argument("--base")
    compare_command.add_argument("--ceiling")
    compare_command.add_argument("--ceiling-dir")
    compare_command.add_argument("--floor")
    compare_command.add_argument("--comment", required=True)
    compare_command.add_argument("--result", required=True)
    record = commands.add_parser("record")
    record.add_argument("--floor")
    record.add_argument("--head", required=True)
    record.add_argument("--sha", required=True)
    record.add_argument("--out", required=True)
    return parser.parse_args(argv)


def main(argv=None):
    args = parse_args(sys.argv[1:] if argv is None else argv)
    if args.command == "bootstrap":
        write_json(args.out, empty_floor())
        return 0
    if args.command == "plan":
        if not SHA.match(args.head):
            fail("--head must be a full commit sha")
        floor = read_floor(args.floor)
        variants = sorted({value for value in (args.base, floor.get("sha")) if value and SHA.match(value)} - {args.head})
        write_json(args.out, {
            "kind": args.kind,
            "head_sha": args.head,
            "base_sha": args.base,
            "floor_sha": floor.get("sha") or None,
            "floor": {
                name: record["sha"] for name, record in floor.get("metrics", {}).items()
            },
            "budgets": floor.get("budgets", {}),
        })
        for sha in variants:
            print(sha)
        return 0
    if args.command == "aggregate":
        raw = Path(args.dir)
        groups = {}
        for path in raw.glob("*-*.json"):
            sha, round_text = path.stem.rsplit("-", 1)
            if not SHA.match(sha) or not round_text.isdigit():
                fail(f"unexpected raw report {path.name}")
            groups.setdefault(sha, {})[int(round_text)] = read_json(path)
        for sha, rounds in groups.items():
            reports = [rounds.get(number) for number in range(1, args.rounds + 1)]
            if any(report is None for report in reports):
                fail(f"{sha} is missing a run")
            write_json(Path(args.out) / f"{sha}.json", aggregate(reports, args.rounds))
        if not groups:
            fail(f"no raw reports found in {raw}")
        return 0
    if args.command == "compare":
        head = validate_aggregate(read_json(args.head), "head")
        base = validate_aggregate(read_json(args.base), "base") if args.base else None
        ceiling = validate_aggregate(read_json(args.ceiling), "ceiling") if args.ceiling else None
        floor = read_floor(args.floor)
        comparison = compare(head, base, ceiling, floor, args.ceiling_dir)
        Path(args.comment).write_text(markdown(comparison), encoding="utf-8")
        Path(args.result).write_text("fail\n" if comparison["failed"] else "pass\n", encoding="utf-8")
        return 1 if comparison["failed"] else 0
    if args.command == "record":
        previous = read_floor(args.floor)
        head = validate_aggregate(read_json(args.head), "head")
        now = datetime.now(timezone.utc).isoformat()
        write_json(args.out, record_floor(previous, head, args.sha, now))
        return 0
    return 2


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (Invalid, OSError, ValueError, KeyError, json.JSONDecodeError) as error:
        print(f"performance: {error}", file=sys.stderr)
        sys.exit(2)
