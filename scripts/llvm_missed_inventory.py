#!/usr/bin/env python3
"""Inventory LLVM missed-optimization issues AND PRs without Search's 1,000 cap.

GitHub content is untrusted data, never executable input. Raw bodies/comments
belong in ignored work/ or external artifacts; the compact CSV is review input,
not a claim that a pattern is reproduced, applicable, or fixed in LCCC.
"""

import argparse
import concurrent.futures
import csv
import hashlib
import io
import json
import re
from pathlib import Path
import subprocess
import threading
import time
from datetime import datetime, timezone
from urllib.parse import urlencode

REPO = "llvm/llvm-project"
LABEL = "missed-optimization"
FIELDS = (
    "number",
    "kind",
    "state",
    "title",
    "url",
    "updated_at",
    "labels",
    "scope",
    "lccc_area",
    "review_status",
)


def classify(item):
    labels = {x["name"].lower() for x in item["labels"]}
    text = (item["title"] + " " + " ".join(labels)).lower()
    # Labels are stronger evidence than prose, but even these are only routing.
    if any("lld" in x for x in labels) or any(
        x in text for x in ("linker", "relocation", "identical code folding")
    ):
        return "linker-review", "src/backend/linker_common; src/bin/lccc_ld.rs"
    unsupported = (
        "amdgpu",
        "nvptx",
        "webassembly",
        "mips",
        "powerpc",
        "hexagon",
        "sparc",
        "systemz",
        "bpf",
        "loongarch",
    )
    if any(x in text for x in unsupported):
        return "unsupported-target-review", "manual mixed-target check required"
    if any(
        x in text
        for x in (
            "flang",
            "fortran",
            "mlir",
            "libc++",
            "c++ coroutine",
            "openmp offload",
        )
    ):
        return (
            "outside-c-frontend-review",
            "manual shared-IR opportunity check required",
        )
    if re.search(r"\b(?:x86(?:_64)?|amd64|avx[0-9a-z]*|sse[0-9a-z]*)\b", text):
        return "supported-target-review", "src/backend/x86"
    if "aarch64" in text or "arm64" in text:
        return "supported-target-review", "src/backend/arm"
    if "riscv" in text or "risc-v" in text:
        return "supported-target-review", "src/backend/riscv"
    if any(x in text for x in ("loop", "vectoriz", "unroll", "licm", "scev", "indvar")):
        return "middle-end-review", "src/passes (loop/vector passes)"
    if any(
        x in text
        for x in (
            "instcombine",
            "simplify",
            "fold",
            "icmp",
            "select",
            "bitwise",
            "shift",
        )
    ):
        return "middle-end-review", "src/passes/simplify.rs; src/passes/bit_idioms.rs"
    return "unclassified-review", "manual review required"


def inventory_csv(items, statuses=None):
    statuses = statuses or {}
    out = io.StringIO(newline="")
    writer = csv.DictWriter(out, fieldnames=FIELDS, lineterminator="\n")
    writer.writeheader()
    for item in sorted(items, key=lambda x: x["number"]):
        scope, area = classify(item)
        row = dict(
            number=item["number"],
            kind="pr" if "pull_request" in item else "issue",
            state=item["state"],
            title=item["title"],
            url=item["html_url"],
            updated_at=item["updated_at"],
            labels=";".join(sorted(x["name"] for x in item["labels"])),
            scope=scope,
            lccc_area=area,
            review_status=statuses.get(item["number"], "unreviewed"),
        )
        # Spreadsheet-safe CSV; preserve the original verbatim in JSONL.
        for key in ("title", "labels"):
            if row[key].lstrip().startswith(("=", "+", "-", "@")):
                row[key] = "'" + row[key]
        writer.writerow(row)
    return out.getvalue()


def enumerate_items(get):
    items = {}
    page = 1
    while True:
        rows = get(
            f"repos/{REPO}/issues?"
            + urlencode(
                dict(
                    state="all",
                    labels=LABEL,
                    sort="created",
                    direction="asc",
                    per_page=100,
                    page=page,
                )
            )
        )
        if not isinstance(rows, list):
            raise ValueError("issues endpoint did not return a list")
        for item in rows:
            number = item["number"]
            if number in items:
                raise ValueError(
                    f"pagination moved or duplicate issue #{number}; retry inventory"
                )
            if LABEL not in {x["name"] for x in item["labels"]}:
                raise ValueError(f"unexpected label membership on #{number}")
            items[number] = item
        if len(rows) < 100:
            break
        page += 1
    if not items:
        raise ValueError("empty inventory; refusing to publish")
    return items


def atomic(path, data):
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(path.name + ".tmp")
    temporary.write_text(data)
    temporary.replace(path)


class GitHub:
    def __init__(self):
        self.lock = threading.Lock()
        self.next_request = 0.0
        self.requests = 0

    def __call__(self, endpoint):
        for attempt in range(4):
            with self.lock:
                pause = self.next_request - time.monotonic()
                if pause > 0:
                    time.sleep(pause)
                self.next_request = time.monotonic() + 0.2
                self.requests += 1
            result = subprocess.run(
                ["gh", "api", endpoint], capture_output=True, text=True, timeout=90
            )
            if not result.returncode:
                return json.loads(result.stdout)
            if attempt < 3 and any(
                x in result.stderr.lower()
                for x in ("rate limit", "http 502", "http 503", "http 504")
            ):
                time.sleep(30 * (attempt + 1))
                continue
            raise RuntimeError(f"GitHub request failed: {endpoint}\n{result.stderr}")
        raise RuntimeError("unreachable retry state")


def comments(get, item):
    rows = []
    if item["comments"]:
        page = 1
        while True:
            chunk = get(
                f"repos/{REPO}/issues/{item['number']}/comments?per_page=100&page={page}"
            )
            if not isinstance(chunk, list):
                raise ValueError("comments endpoint did not return a list")
            rows.extend(chunk)
            if len(chunk) < 100:
                break
            page += 1
    ids = [x["id"] for x in rows]
    if len(ids) != len(set(ids)):
        raise ValueError(f"incomplete/moving comment collection for #{item['number']}")
    return {
        "number": item["number"],
        "expected_comments": item["comments"],
        "counter_gap": max(0, item["comments"] - len(rows)),
        "comments": rows,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--output",
        type=Path,
        required=True,
        help="raw artifact directory (normally work/)",
    )
    parser.add_argument("--csv", type=Path, required=True, help="compact review index")
    parser.add_argument(
        "--comments",
        action="store_true",
        help="also collect issue/PR conversation comments; not PR reviews/diffs",
    )
    parser.add_argument(
        "--resume",
        action="store_true",
        help="resume comments against the original hashed post snapshot",
    )
    parser.add_argument(
        "--ledger",
        type=Path,
        help="checked-in batch ledger with explicit limited coverage",
    )
    args = parser.parse_args()
    if args.resume and not args.comments:
        parser.error("--resume requires --comments")
    started = datetime.now(timezone.utc).isoformat()
    manifest = {
        "repository": REPO,
        "label": LABEL,
        "started_at": started,
        "status": "incomplete",
        "method": "REST Issues state=all (includes PRs); two stable membership enumerations; no Search API cap",
        "limitations": [
            "Not an atomic GitHub snapshot; membership must agree in two enumerations.",
            "Label applies to current membership, not removed historical labels.",
            "PR review threads, diffs, linked external pages and removed comments are not conversation comments.",
            "Automatic scope classification is routing only; every row starts unreviewed.",
        ],
    }
    get = GitHub()
    if args.resume:
        manifest = json.loads((args.output / "manifest.json").read_text())
        raw = (args.output / "posts.jsonl").read_text()
        if hashlib.sha256(raw.encode()).hexdigest() != manifest["posts_sha256"]:
            raise ValueError("post snapshot hash mismatch; refusing resume")
        items = [json.loads(line) for line in raw.splitlines()]
    else:
        atomic(args.output / "manifest.json", json.dumps(manifest, indent=2) + "\n")
        first = enumerate_items(get)
        second = enumerate_items(get)
        if set(first) != set(second):
            raise ValueError(
                "label membership changed between enumerations; rerun rather than claim completeness"
            )
        items = list(second.values())
        raw = "".join(
            json.dumps(x, sort_keys=True) + "\n"
            for x in sorted(items, key=lambda x: x["number"])
        )
        atomic(args.output / "posts.jsonl", raw)
        atomic(args.csv, inventory_csv(items))
        manifest.update(
            posts=len(items),
            issues=sum("pull_request" not in x for x in items),
            prs=sum("pull_request" in x for x in items),
            requests=get.requests,
            posts_sha256=hashlib.sha256(raw.encode()).hexdigest(),
            csv_sha256=hashlib.sha256(args.csv.read_bytes()).hexdigest(),
        )
        atomic(args.output / "manifest.json", json.dumps(manifest, indent=2) + "\n")
    statuses = {}
    if args.ledger:
        ledger = json.loads(args.ledger.read_text())["cases"]
        statuses = {case["number"]: case["status"] for case in ledger}
        if len(statuses) != len(ledger) or not set(statuses).issubset(
            {x["number"] for x in items}
        ):
            raise ValueError("duplicate/unknown ledger IDs")
        if not set(statuses.values()).issubset(
            {"covered-limited", "reviewed-deferred", "unreviewed"}
        ):
            raise ValueError("unknown coverage status")
    # A failed derivation or resume must never leave a manifest claiming the
    # newly requested artifact set is complete.
    manifest["status"] = "incomplete"
    atomic(args.output / "manifest.json", json.dumps(manifest, indent=2) + "\n")
    atomic(args.csv, inventory_csv(items, statuses))
    manifest["csv_sha256"] = hashlib.sha256(args.csv.read_bytes()).hexdigest()
    if args.ledger:
        manifest["ledger_sha256"] = hashlib.sha256(args.ledger.read_bytes()).hexdigest()
    print(f"Enumerated {len(items)} posts; compact inventory published", flush=True)
    if args.comments:
        # Write each completed conversation immediately. A crash leaves an
        # explicitly incomplete manifest plus a resumable/auditable partial log.
        path = args.output / "comments.jsonl"
        if args.resume and manifest.get("comments_sha256"):
            if (
                hashlib.sha256(path.read_bytes()).hexdigest()
                != manifest["comments_sha256"]
            ):
                raise ValueError("conversation snapshot hash mismatch; refusing resume")
        prior = (
            [json.loads(line) for line in path.read_text().splitlines()]
            if args.resume and path.exists()
            else []
        )
        done = {record["number"] for record in prior}
        if len(done) != len(prior) or not done.issubset({x["number"] for x in items}):
            raise ValueError("invalid/duplicate resume conversations")
        pending = [x for x in items if x["number"] not in done]
        gaps = [
            {"number": x["number"], "missing_from_api_counter": x["counter_gap"]}
            for x in prior
            if x.get("counter_gap")
        ]
        with path.open("a" if args.resume else "w") as stream:
            with concurrent.futures.ThreadPoolExecutor(max_workers=4) as pool:
                for index, record in enumerate(
                    pool.map(lambda x: comments(get, x), pending), len(prior) + 1
                ):
                    stream.write(json.dumps(record, sort_keys=True) + "\n")
                    stream.flush()
                    if record["counter_gap"]:
                        gaps.append(
                            {
                                "number": record["number"],
                                "missing_from_api_counter": record["counter_gap"],
                            }
                        )
                    if index % 100 == 0:
                        print(f"Conversations {index}/{len(items)}", flush=True)
        manifest["comment_counter_gaps"] = gaps
        manifest["comments_sha256"] = hashlib.sha256(
            (args.output / "comments.jsonl").read_bytes()
        ).hexdigest()
    manifest.update(
        status="complete",
        completed_at=datetime.now(timezone.utc).isoformat(),
        requests_this_invocation=get.requests,
    )
    atomic(args.output / "manifest.json", json.dumps(manifest, indent=2) + "\n")
    print("Complete", flush=True)


if __name__ == "__main__":
    main()
