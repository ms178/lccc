#!/usr/bin/env python3
"""Publish auditable same-host native/portable research receipts, not a speed gate.

Checks annotations remain accessible when artifact-storage downloads are blocked.
Raw samples, binaries and disassembly remain in the uploaded research artifact.
Never infer a before/after gain from a vendor comparison on another CPU.
"""
from __future__ import annotations

import argparse
import hashlib
import json
from pathlib import Path
import re
import sys


# Bounded excerpts of the actual timed binaries, not a separate compiler run.
# GCC may give a specialized clone a suffix such as .constprop.0. The complete
# disassemblies and the exact binaries remain in the research artifact.
PRIORITY_SYMBOLS = {
    "zlib_ng_slide_hash": ("slide_hash_c_chain", "zng_slide_hash_c", "zng_insert_string_batch"),
    "fp64_product_map": ("fp_product_map", "main"),
    "fp32_product_map": ("fp_product_map", "main"),
    "byte_load_reassembly": ("main",),
    "linux_list_sort": ("merge_and_count", "merge", "list_sort"),
    "sqlite_vdbe_serial": ("vdbe_serial_get", "scan_all_records"),
    "glibc_strstr": ("two_way_short_needle", "main"),
    "zlib_ng_compare256": ("zlib_compare256_64",),
    "arith_loop": ("arith_loop",),
}


def symbol_excerpt(disassembly: str, names: tuple[str, ...], limit: int = 7000) -> str:
    """Return named objdump functions, including compiler-suffixed clones.

    A missing symbol is reported rather than replaced by an unrelated routine;
    it may have been inlined. Excerpts explicitly say when they were truncated.
    """
    header = re.compile(r"^[0-9a-fA-F]+ <([^>]+)>:\s*$")
    selected = []
    keep = False
    for line in disassembly.splitlines():
        match = header.match(line)
        if match:
            symbol = match.group(1)
            keep = any(symbol == name or symbol.startswith(name + ".") for name in names)
        elif line.startswith("Disassembly of section "):
            keep = False
        if keep:
            selected.append(line)
    if not selected:
        return "No named function found (it may have been inlined)."
    text = "\n".join(selected)
    if len(text) > limit:
        text = text[:limit] + "\n[excerpt truncated; full disassembly is in the artifact]"
    return text


def summaries(report: dict, profile: str, expected_cases: set[str] | None = None,
              expected_compilers: set[str] | None = None) -> list[dict]:
    if report.get("complete") is not True:
        raise ValueError("incomplete report")
    if report.get("unavailable_compilers"):
        raise ValueError("requested compiler unavailable")
    settings = report["settings"]
    flags = settings["common_cflags"]
    if settings["opt"] != "-O2" or settings["reps"] < 9 or settings["warmup"] < 3:
        raise ValueError("insufficient or foreign timing settings")
    if ("-march=native" in flags) != (profile == "native"):
        raise ValueError("profile label does not match compiler flags")
    if not report["metadata"]["pinning"]["applied"]:
        raise ValueError("CPU affinity was not applied")
    identities = {c["key"]: c["binary_sha256"] for c in report["compilers"]}
    if len(identities) != len(report["compilers"]):
        raise ValueError("duplicate compiler identity")
    if not {"lccc", "before", "gcc", "clang"} <= identities.keys():
        raise ValueError("missing before/current/vendor compiler")
    if expected_compilers is not None and set(identities) != expected_compilers:
        raise ValueError("reported compiler selection does not match the requested selection")
    rows = []
    names = set()
    for b in report["benchmarks"]:
        if b["name"] in names:
            raise ValueError("duplicate workload")
        names.add(b["name"])
        for arm in identities:
            c = b["correctness"][arm]
            if not all(c[k] for k in ("all_runs_successful", "matches_baseline", "stable_within_compiler")):
                raise ValueError(f"incorrect workload: {b['name']}/{arm}")
            if b["runtime"][arm]["wall"]["n"] != settings["reps"]:
                raise ValueError("incomplete paired rounds")
        ratios = {}
        for ref in ("before", "gcc", "clang", "icx", "rolled"):
            if ref in identities:
                r = b["ratios_from_lccc"][ref]
                ratios[ref] = {"median": r["median"], "ci95": r["median_ci95_bootstrap"]}
        row = {
            "profile": profile, "name": b["name"], "ratios_after_over": ratios,
            "whole_executable_identical": b["compile"]["lccc"]["binary_sha256"] == b["compile"]["before"]["binary_sha256"],
            "source_sha256": b["source_sha256"],
        }
        if "rolled" in identities:
            row["whole_executable_identical_rolled"] = (
                b["compile"]["lccc"]["binary_sha256"] == b["compile"]["rolled"]["binary_sha256"]
            )
        rows.append(row)
    if not rows:
        raise ValueError("empty report")
    if expected_cases is not None and names != expected_cases:
        raise ValueError("reported workload selection does not match the requested selection")
    return rows


def rejection_details(report: object, limit: int = 8) -> str:
    """Summarize failures without replacing the full report or relaxing the gate.

    A checks annotation survives unavailable artifact storage, but has a hard
    size limit. Preserve multiple independent errors and bound each untrusted
    diagnostic (including compiler stderr and program output) before encoding.
    """
    if not isinstance(report, dict):
        return ""
    details: list[str] = []

    def short(value: object, width: int = 130) -> str:
        text = " ".join(str(value)[:1000].split()).encode("ascii", "backslashreplace").decode("ascii")
        text = text.replace("\\", "/").replace('"', "'")
        return text if len(text) <= width else text[:width] + "..."

    def add(message: str) -> None:
        if message not in details:
            details.append(message)

    for arm, reason in report.get("unavailable_compilers", {}).items():
        add(f"compiler {short(arm, 40)} unavailable: {short(reason)}")
    for item in report.get("benchmarks", []):
        name = short(item.get("name", "unknown"), 50)
        failed_compiles = set()
        for arm, result in item.get("compile", {}).items():
            if result.get("ok") is False:
                failed_compiles.add(arm)
                reason = result.get("error") or result.get("stderr") or f"exit={result.get('returncode')}"
                add(f"{name}/{short(arm, 40)} compile: {short(reason)}")
        checks = item.get("correctness", {})
        baseline = item.get("baseline_compiler")
        for arm, check in checks.items():
            if arm in failed_compiles:
                continue
            label = f"{name}/{short(arm, 40)}"
            if check.get("all_runs_successful") is not True:
                runs = item.get("warmups", {}).get(arm, []) + item.get("runs", {}).get(arm, [])
                failed = next((run for run in runs if not run.get("ok")), {})
                reason = failed.get("error") or failed.get("stderr") or f"exit={failed.get('returncode')}"
                add(f"{label} execution: {short(reason)}")
            elif check.get("stable_within_compiler") is not True:
                add(f"{label} output unstable")
            elif check.get("matches_baseline") is not True:
                oracle = checks.get(baseline, {}).get("output")
                add(f"{label} output mismatch: got [{short(check.get('output'), 80)}]; "
                    f"{short(baseline, 40)} [{short(oracle, 80)}]")
        for warning in item.get("warnings", []):
            if "failure" in str(warning).lower() or "not found" in str(warning).lower():
                add(f"{name}: {short(warning)}")
    if not details:
        return ""
    return json.dumps({"details": details[:limit], "omitted": max(0, len(details) - limit)},
                      ensure_ascii=True, separators=(",", ":"))


def optimization_controls(root: Path, selected: set[str]) -> dict:
    """Collect bounded same-compiler A/Bs; absent or partial files stay explicit.

    Timings exclude process startup and compare enabled code against the same
    compiler with one existing optimization control disabled. Callgrind Ir is
    a separate simulated metric, not elapsed time or a claim about another CPU.
    """
    controls = {}
    for control, case in (("strided", "byte_load_reassembly"), ("string_inline", "glibc_strstr")):
        if case not in selected:
            continue
        result = {"case": case, "reference": "disabled", "profiles": {}}
        for profile in ("portable", "native"):
            path = root / f"{profile}-{control}-ab" / "results.json"
            if not path.exists():
                result["profiles"][profile] = {"available": False}
                continue
            raw = path.read_bytes()
            data = json.loads(raw)
            entry = {"complete": False, "sha256": hashlib.sha256(raw).hexdigest()}
            rows = data.get("rows", [])
            if (data.get("complete") is True and data.get("pinning", {}).get("applied") is True
                    and data.get("reference") == "disabled" and len(rows) == 1
                    and rows[0].get("name") == case and len(rows[0].get("samples", [])) == data.get("rounds")
                    and isinstance(data.get("rounds"), int) and data["rounds"] >= 9):
                comparisons = rows[0]["comparisons"]
                entry.update(complete=True, rounds=data["rounds"], ratios={
                    arm: {"median": comparisons[arm]["median"],
                          "ci95": comparisons[arm]["exploratory_ci95"],
                          "identical": comparisons[arm]["identical_binary_to_reference"],
                          "min_sample_s": comparisons[arm]["minimum_sample_seconds"]}
                    for arm in ("lccc", "gcc")})
            result["profiles"][profile] = entry
        path = root / f"callgrind-{control}-ab" / "index.json"
        if not path.exists():
            result["callgrind"] = {"available": False}
        else:
            raw = path.read_bytes()
            data = json.loads(raw)
            complete = (data.get("complete") is True and not data.get("failures")
                        and set(data.get("results", {})) == {case})
            result["callgrind"] = {"complete": complete, "sha256": hashlib.sha256(raw).hexdigest()}
            if complete:
                result["callgrind"]["ir_ratio"] = data["results"][case]["metrics"]["ir_ratio"]
        controls[control] = result
    return controls


def row_notices(rows: list[dict], limit: int = 3900) -> list[str]:
    """Pack runtime rows into as few annotations as the byte limit allows.

    Notices are a scarce per-job resource, so a fixed five-rows-per-notice grid
    both wastes them on short rows and fails outright on a large one. Pack
    greedily instead, and keep every emitted notice inside the hard limit.
    """
    def encode(chunk: list[dict]) -> str:
        return json.dumps(chunk, separators=(",", ":"))

    notices: list[str] = []
    chunk: list[dict] = []
    for row in rows:
        if chunk and len(encode([*chunk, row]).encode()) > limit:
            notices.append(encode(chunk))
            chunk = []
        chunk.append(row)
        if len(encode(chunk).encode()) > limit:
            raise ValueError("single runtime row exceeds the annotation budget")
    if chunk:
        notices.append(encode(chunk))
    return notices


def control_digest(controls: dict) -> dict:
    """Compact medians for the primary evidence notice.

    The complete control records (sample counts, hashes, interval bounds,
    minimum durations) stay in `summary.json`, the step summary and the
    artifact; the notice keeps only what a reviewer needs to see the shape.
    """
    digest = {}
    for control, result in controls.items():
        profiles = {}
        for profile, entry in result.get("profiles", {}).items():
            if entry.get("complete"):
                profiles[profile] = {
                    "rounds": entry["rounds"],
                    "ratios": {arm: [round(value["median"], 4), value["identical"]]
                               for arm, value in entry["ratios"].items()},
                }
            else:
                profiles[profile] = {"available": False}
        callgrind = result.get("callgrind", {})
        digest[control] = {
            "case": result["case"],
            "profiles": profiles,
            "callgrind_ir": (round(callgrind["ir_ratio"], 5)
                             if callgrind.get("complete") and "ir_ratio" in callgrind else None),
        }
    return digest


def annotate(title: str, text: str) -> None:
    escaped = text.replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")
    print(f"::notice title={title}::{escaped}")


def focused_excerpt(disassembly: str, symbol: str, limit: int, focus: str | None = None) -> str:
    """Keep a bounded selected loop or caller window, with an explicit fallback."""
    full = symbol_excerpt(disassembly, (symbol,), limit=1_000_000)
    if full.startswith("No named function") or not focus:
        return symbol_excerpt(disassembly, (symbol,), limit=limit)
    lines = full.splitlines()
    positions = []
    branch_spans = {}
    if focus == "call":
        positions = [i for i, line in enumerate(lines)
                     if re.search(r"\bcallq?\b.*<fp_product_map(?:\.|>)", line)]
    elif focus == "narrow-be":
        positions = [i for i, line in enumerate(lines)
                     if re.search(r"\b(?:movsw[ql]|movbe\w*|rol\w*)\b", line)]
    else:
        for i, line in enumerate(lines):
            branch = re.search(r"^\s*([0-9a-fA-F]+):.*\s+j[a-z]+\s+([0-9a-fA-F]+)\s*<", line)
            if branch and int(branch.group(2), 16) < int(branch.group(1), 16):
                positions.append(i)
                branch_spans[i] = int(branch.group(1), 16) - int(branch.group(2), 16)
    if not positions:
        return "[focused site not found; showing function start]\n" + symbol_excerpt(
            disassembly, (symbol,), limit=max(0, limit - 60))
    if focus == "shortest-loop":
        at = min(positions, key=branch_spans.__getitem__)
    else:
        at = positions[0] if focus in ("call", "first-loop", "narrow-be") else positions[-1]
    before = 7 if focus == "call" else 5 if focus == "narrow-be" else 14
    after = 11 if focus == "call" else 6 if focus == "narrow-be" else 4
    header = [lines[0]]
    if focus == "shortest-loop":
        header.append("[shortest backward branch; not a measured hotness claim]")
    excerpt = "\n".join([*header, *lines[max(1, at - before):at + after]])
    if len(excerpt.encode()) > limit:
        excerpt = excerpt.encode()[:limit].decode("utf-8", errors="ignore") + "\n[window truncated]"
    return excerpt


def publish_disassembly(root: Path, profile: str, cases: list[str]) -> None:
    """Publish comparison windows that fit both per-message and job limits.

    Checks annotations are capped at 4096 bytes *each*, as well as a bounded
    count per job. Put related arms in the same small notice; large complete
    disassemblies and exact binaries remain in the research artifact.
    """
    for name in cases:
        if name == "nbody":
            groups = [("sqrt", (), ("lccc", "before", "gcc", "clang"), "sqrt")]
        elif name == "zlib_ng_slide_hash":
            groups = [
                ("saturation", ("slide_hash_c_chain", "zng_slide_hash_c"),
                 ("lccc", "gcc", "icx"), None),
                ("insertion", ("zng_insert_string_batch",), ("lccc", "gcc"), "last-loop"),
            ]
        elif name in ("fp32_product_map", "fp64_product_map"):
            groups = [
                ("kernel", ("fp_product_map",), ("lccc", "gcc", "icx", "rolled"), "first-loop"),
                ("caller", ("main",), ("lccc", "before", "gcc"), "call"),
            ]
        elif name == "sqlite_vdbe_serial":
            groups = [("narrow-be", ("vdbe_serial_get",),
                       ("lccc", "before", "gcc"), "narrow-be")]
        elif name == "glibc_strstr":
            groups = [("shortest-backedge", PRIORITY_SYMBOLS[name], ("lccc", "gcc"), "shortest-loop")]
        else:
            focus = "last-loop" if name == "byte_load_reassembly" else None
            groups = [("kernel", PRIORITY_SYMBOLS[name], ("lccc", "gcc"), focus)]
        for part, symbols, arms, focus in groups:
            sections = []
            # Reserve space for section headers and truncation notices; never
            # depend on GitHub silently clipping a later comparison arm.
            limit = 3450 // len(arms)
            for arm in arms:
                path = root / f"{profile}-artifacts" / name / arm / "assembly.txt"
                if not path.exists():
                    continue
                disassembly = path.read_text()
                if focus == "sqrt":
                    lines = disassembly.splitlines()
                    positions = [i for i, line in enumerate(lines) if "sqrtss" in line or "sqrtsd" in line]
                    if positions:
                        i = positions[0]
                        text = "\n".join(lines[max(0, i - 4):i + 6])
                        text = text.encode()[:limit].decode("utf-8", errors="ignore")
                    else:
                        text = "No scalar sqrt found."
                    symbol = "sqrt"
                else:
                    symbol = symbols[0]
                    text = "No named function found (it may have been inlined)."
                    for candidate in symbols:
                        candidate_text = focused_excerpt(disassembly, candidate, limit, focus)
                        if not candidate_text.startswith("No named function"):
                            symbol, text = candidate, candidate_text
                            break
                sections.append(f"=== {arm}/{symbol} ===\n{text}")
            if sections:
                text = "\n\n".join(sections)
                if len(text.encode()) > 3900:
                    raise ValueError(f"assembly notice exceeds byte budget: {profile}/{name}/{part}")
                annotate(f"CPU disassembly {profile}/{name}/{part}", text)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("root", type=Path)
    parser.add_argument("--assembly-profile", choices=("portable", "native"))
    parser.add_argument("--assembly-cases", help="comma-separated named workloads")
    parser.add_argument("--expected-cases", help="requested comma-separated workloads, or all")
    parser.add_argument("--expected-compilers", help="requested comma-separated compiler keys")
    args = parser.parse_args()
    root = args.root
    if args.assembly_profile or args.assembly_cases:
        if not args.assembly_profile or not args.assembly_cases:
            parser.error("assembly publication requires both profile and cases")
        cases = args.assembly_cases.split(",")
        if any(case not in PRIORITY_SYMBOLS and case != "nbody" for case in cases):
            parser.error("unknown assembly workload")
        publish_disassembly(root, args.assembly_profile, cases)
        return 0
    expected_cases = None
    if args.expected_cases == "all":
        sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
        from tests.benchmark.run_benchmarks import BENCHMARKS
        expected_cases = {b.name for b in BENCHMARKS}
    elif args.expected_cases is not None:
        expected_cases = {name.strip() for name in args.expected_cases.split(",") if name.strip()}
        if not expected_cases:
            parser.error("expected workload selection is empty")
    expected_compilers = None
    if args.expected_compilers is not None:
        expected_compilers = {name.strip() for name in args.expected_compilers.split(",") if name.strip()}
        if not expected_compilers:
            parser.error("expected compiler selection is empty")
    failed = False
    reports = {}
    for profile in ("portable", "native"):
        path = root / f"{profile}.json"
        if not path.exists():
            annotate("CPU runtime report missing", profile)
            failed = True
            continue
        report = None
        try:
            report = json.loads(path.read_text())
            rows = summaries(report, profile, expected_cases, expected_compilers)
            metadata = report["metadata"]
            identity = {"profile": profile, "cpu": metadata["cpu_models"],
                        "settings": report["settings"], "pinning": metadata["pinning"],
                        "compiler_versions": metadata["compiler_versions"],
                        "report_sha256": hashlib.sha256(path.read_bytes()).hexdigest()}
            annotate("CPU runtime identity", json.dumps(identity, separators=(",", ":")))
            for chunk in row_notices(rows):
                annotate("CPU paired runtime", chunk)
            reports[profile] = report
        except (ValueError, KeyError) as exc:
            reason = str(exc).encode("ascii", "backslashreplace").decode("ascii")[:300]
            text = f"{profile}: {reason}"
            details = rejection_details(report)
            if details:
                text += "\n" + details
            if len(text.encode()) > 3900:
                raise ValueError("rejection notice exceeds the annotation budget")
            annotate("CPU runtime report rejected", text)
            failed = True
    if len(reports) == 2:
        a, b = reports["portable"], reports["native"]
        for key in ("cpu_models", "pinning"):
            if a["metadata"][key] != b["metadata"][key]:
                annotate("CPU runtime report rejected", "profiles: CPU/affinity differs")
                failed = True
                break
        if [(x["name"], x["source_sha256"]) for x in a["benchmarks"]] != [(x["name"], x["source_sha256"]) for x in b["benchmarks"]]:
            annotate("CPU runtime report rejected", "profiles: workloads/sources changed")
            failed = True
    return int(failed)


if __name__ == "__main__":
    raise SystemExit(main())
