#!/usr/bin/env python3
"""Prepare, run, and report the M3 20-node physical fan-out token measurement.

This is a one-off reproducibility helper for docs/benchmarks artifacts. It runs
one fresh Codex CLI session per DAG node and sums provider-reported token usage.
"""

from __future__ import annotations

import argparse
import json
import statistics
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

ROOT = Path(__file__).resolve().parents[1]
GRAPH_PATH = ROOT / "docs/execution_plans/m3_one_hour_sprint_20_node_measured_run.task_graph.json"
SOURCE_PLAN_PATH = ROOT / "docs/execution_plans/m3_one_hour_sprint_20_node_measured_run.md"
SINGLE_PACKET_REPORT = ROOT / "docs/benchmarks/m3_one_hour_sprint_20_node_measured_20260423_report.json"
PROMPTS_DIR = ROOT / "docs/benchmarks/m3_one_hour_sprint_20_node_physical_fanout_prompts"
RUN_DIR = ROOT / "docs/benchmarks/m3_one_hour_sprint_20_node_physical_fanout_runs"
PACKETS_MD = ROOT / "docs/benchmarks/m3_one_hour_sprint_20_node_physical_fanout_prompt_packets.md"
MANIFEST_PATH = ROOT / "docs/benchmarks/m3_one_hour_sprint_20_node_physical_fanout_20260423.json"
OUTPUTS_PATH = ROOT / "docs/benchmarks/m3_one_hour_sprint_20_node_physical_fanout_20260423_outputs.json"
REPORT_JSON = ROOT / "docs/benchmarks/m3_one_hour_sprint_20_node_physical_fanout_20260423_report.json"
REPORT_MD = ROOT / "docs/benchmarks/m3_one_hour_sprint_20_node_physical_fanout_20260423_report.md"
CODEX = Path.home() / ".local/bin/codex"
MODEL = "gpt-5.4"
PROVIDER = "openai"
SNAPSHOT = "SNP-06608A70A040"


def _load_json(path: Path) -> Any:
    return json.loads(path.read_text(encoding="utf-8"))


def _write_json(path: Path, payload: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(payload, indent=2, sort_keys=False) + "\n", encoding="utf-8")


def _node_id(node: dict[str, Any]) -> str:
    return str(node.get("node_id") or node.get("id") or "")


def _graph_summary(nodes: list[dict[str, Any]]) -> str:
    lines = []
    for node in nodes:
        deps = node.get("depends_on") or []
        dep_text = ",".join(deps) if deps else "root"
        weight = node.get("progress_weight", 1)
        lines.append(f"{_node_id(node)} ({dep_text}, weight={weight}): {node.get('title')}")
    return "\n".join(lines)


def _prompt_for_node(node: dict[str, Any], nodes: list[dict[str, Any]]) -> str:
    node_id = _node_id(node)
    deps = node.get("depends_on") or []
    deps_text = ", ".join(deps) if deps else "none/root"
    kind = node.get("node_kind") or "task"
    progress_weight = node.get("progress_weight", 1)
    template = node.get("task_template") or {}
    graph_summary = _graph_summary(nodes)
    completion_rule = node.get("completion_rule") or "produce a concise comparable node handoff; do not claim product-level savings"
    return f"""You are running a provider-measured M3 20-node physical fan-out trial for ait.

Hard controls:
- Fresh Codex session for exactly one DAG node.
- Do not run shell commands, inspect files, browse, or call tools; use only this packet.
- Produce final answer only, concise and review-ready.
- This is benchmark evidence, not a product claim.
- Preserve the repository governance boundary: docs/plan.md is constitutional, legal-layer docs win over command-layer docs, and ait.md workflow gates remain explicit.

Measurement boundary:
- Existing single prompt-packet M3 20-node measured proof point used 21,681 total tokens for ait_dag.
- This physical fan-out trial runs each DAG node as a separate fresh provider session and sums the node usage.
- Compare only after all 20 node sessions are complete.
- The result may show overhead; do not force a positive savings statement.

Full 20-node graph summary:
{graph_summary}

Current node package:
- run_id: m3-20-physical-node-{node_id.lower()}-measured-01
- node_id: {node_id}
- node_kind: {kind}
- title: {node.get('title')}
- plan_item_ref: {node.get('plan_item_ref')}
- depends_on: {deps_text}
- progress_weight: {progress_weight}
- task_template_title: {template.get('title') or 'n/a'}
- change_title: {template.get('change_title') or 'n/a'}
- completion_rule: {completion_rule}

Output requirements:
1. State this node's role and dependency assumptions.
2. State whether the node can be considered quality-passed for a physical fan-out measurement.
3. Include any blocker or caveat.
4. Include the next DAG node(s) that become unblocked if this node passes.
5. Include provider-usage import instruction: record this session's provider token usage under run_id m3-20-physical-node-{node_id.lower()}-measured-01.
6. Keep claim wording conservative: this node alone proves nothing; only the 20-session aggregate can be compared.
"""


def _baselines() -> dict[str, Any]:
    report = _load_json(SINGLE_PACKET_REPORT)
    runs = report["workloads"][0]["runs"]
    by_mode = {run["mode"]: run for run in runs}
    return {
        "source_report": str(SINGLE_PACKET_REPORT.relative_to(ROOT)),
        "git_linear": by_mode["git_linear"]["total_tokens"],
        "ait_linear": by_mode["ait_linear"]["total_tokens"],
        "ait_dag_single_packet": by_mode["ait_dag"]["total_tokens"],
        "ait_dag_single_packet_savings_percent": report["aggregate"]["long_ait_dag_median_saving_percent"],
    }


def prepare() -> None:
    graph = _load_json(GRAPH_PATH)
    nodes = graph["nodes"]
    PROMPTS_DIR.mkdir(parents=True, exist_ok=True)
    RUN_DIR.mkdir(parents=True, exist_ok=True)

    manifest_nodes = []
    packet_lines = [
        "# M3 20-Node Physical Fan-Out Prompt Packets",
        "",
        "Authority: command layer under [../plan.md](../plan.md) and the legal-layer governance documents.",
        "Status: current prompt-packet companion for the M3 20-node physical fan-out measured run.",
        "Scope: exact per-node prompt packets for running one fresh Codex provider session per M3 DAG node and summing node usage.",
        "",
        "This packet set is intentionally different from `m3_one_hour_sprint_20_node_measured_prompt_packets.md`, which measures a single compact `ait_dag` prompt packet.",
        "The physical fan-out run measures execution-level overhead by running all 20 DAG nodes as separate fresh sessions.",
        "",
        "## Common controls",
        "",
        "- Use provider `openai`, model `gpt-5.4`, read-only sandbox, no web, no tool/file reads, and fresh session state per node.",
        "- Record provider-reported token usage from each Codex session JSONL.",
        "- Sum all 20 node usages for the physical fan-out total.",
        "- Do not use this single M3 workload as standalone product-claim evidence.",
        "",
    ]

    for node in nodes:
        node_id = _node_id(node)
        run_id = f"m3-20-physical-node-{node_id.lower()}-measured-01"
        prompt = _prompt_for_node(node, nodes)
        prompt_path = PROMPTS_DIR / f"node_{node_id}.prompt.txt"
        prompt_path.write_text(prompt, encoding="utf-8")
        packet_lines.extend([
            f"## Node {node_id} - {node.get('title')}",
            "",
            f"Run ID: `{run_id}`",
            "",
            f"Prompt file: [`{prompt_path.relative_to(ROOT)}`](./m3_one_hour_sprint_20_node_physical_fanout_prompts/{prompt_path.name})",
            "",
            "```text",
            prompt.strip(),
            "```",
            "",
        ])
        manifest_nodes.append(
            {
                "run_id": run_id,
                "node_id": node_id,
                "node_kind": node.get("node_kind"),
                "title": node.get("title"),
                "plan_item_ref": node.get("plan_item_ref"),
                "depends_on": node.get("depends_on") or [],
                "progress_weight": node.get("progress_weight", 1),
                "usage_kind": "measured",
                "quality": "pending",
                "prompt_file": str(prompt_path.relative_to(ROOT)),
                "usage": {
                    "prompt_tokens": None,
                    "completion_tokens": None,
                    "total_tokens": None,
                    "cached_input_tokens": None,
                    "reasoning_output_tokens": None,
                },
                "session_jsonl": None,
                "output_file": None,
                "notes": "pending fresh Codex provider session",
            }
        )

    manifest = {
        "benchmark_id": "m3-one-hour-sprint-20-node-physical-fanout-20260423",
        "description": "Physical fan-out measurement for the M3 20-node DAG: one fresh Codex provider session per node, summed usage compared with the prior single prompt-packet measured run.",
        "measurement_type": "physical_fanout_sum",
        "provider": PROVIDER,
        "model": MODEL,
        "repository_snapshot_reference": SNAPSHOT,
        "source_graph": str(GRAPH_PATH.relative_to(ROOT)),
        "source_plan": str(SOURCE_PLAN_PATH.relative_to(ROOT)),
        "prompt_packets": str(PACKETS_MD.relative_to(ROOT)),
        "baselines": _baselines(),
        "claim_boundary": "Single M3 workload; execution-level overhead proof point only; not standalone product-claim evidence.",
        "nodes": manifest_nodes,
    }
    PACKETS_MD.write_text("\n".join(packet_lines), encoding="utf-8")
    _write_json(MANIFEST_PATH, manifest)
    if not OUTPUTS_PATH.exists():
        _write_json(OUTPUTS_PATH, {"benchmark_id": manifest["benchmark_id"], "runs": []})


def _extract_usage(session_jsonl: Path) -> dict[str, Any]:
    sys.path.insert(0, str(ROOT / "src"))
    from ait.token_benchmark import extract_codex_token_usage

    return extract_codex_token_usage(session_jsonl)


def _latest_session_after(stamp: float) -> Path:
    root = Path.home() / ".codex/sessions"
    candidates = [p for p in root.rglob("*.jsonl") if p.stat().st_mtime >= stamp]
    if not candidates:
        raise RuntimeError("No Codex session JSONL found after run stamp")
    return max(candidates, key=lambda p: p.stat().st_mtime)


def _save_manifest(manifest: dict[str, Any]) -> None:
    _write_json(MANIFEST_PATH, manifest)


def run(limit: int | None = None) -> None:
    if not MANIFEST_PATH.exists():
        prepare()
    manifest = _load_json(MANIFEST_PATH)
    outputs = _load_json(OUTPUTS_PATH) if OUTPUTS_PATH.exists() else {"benchmark_id": manifest["benchmark_id"], "runs": []}
    existing_outputs = {item.get("run_id"): item for item in outputs.get("runs", [])}
    completed = 0
    for node in manifest["nodes"]:
        if node.get("quality") == "passed" and node.get("usage", {}).get("total_tokens"):
            continue
        if limit is not None and completed >= limit:
            break
        run_id = node["run_id"]
        node_id = node["node_id"]
        prompt_path = ROOT / node["prompt_file"]
        out_file = RUN_DIR / f"{node_id}_last_message.txt"
        stdout_file = RUN_DIR / f"{node_id}_codex_events.jsonl"
        stderr_file = RUN_DIR / f"{node_id}_codex_stderr.log"
        prompt = prompt_path.read_text(encoding="utf-8")
        stamp = time.time()
        cmd = [
            str(CODEX),
            "exec",
            "--model",
            MODEL,
            "--sandbox",
            "read-only",
            "--skip-git-repo-check",
            "--cd",
            str(ROOT),
            "--color",
            "never",
            "--json",
            "--output-last-message",
            str(out_file),
            "-c",
            'web_search="disabled"',
            "-c",
            'approval_policy="never"',
        ]
        print(f"running {run_id} ...", flush=True)
        with stdout_file.open("w", encoding="utf-8") as stdout, stderr_file.open("w", encoding="utf-8") as stderr:
            proc = subprocess.run(cmd, input=prompt, text=True, stdout=stdout, stderr=stderr, timeout=900, cwd=ROOT)
        with stderr_file.open("a", encoding="utf-8") as stderr:
            stderr.write(f"node={node_id} run_id={run_id}\n")
        if proc.returncode != 0:
            node["quality"] = "failed"
            node["notes"] = f"Codex exec failed with return code {proc.returncode}; see {stderr_file.relative_to(ROOT)}"
            _save_manifest(manifest)
            raise RuntimeError(node["notes"])
        session_jsonl = _latest_session_after(stamp)
        usage_payload = _extract_usage(session_jsonl)
        usage = usage_payload["manifest_usage"]
        output_text = out_file.read_text(encoding="utf-8") if out_file.exists() else ""
        node["quality"] = "passed"
        node["usage"] = usage
        node["session_jsonl"] = str(session_jsonl)
        node["output_file"] = str(out_file.relative_to(ROOT))
        node["notes"] = (
            f"Fresh Codex CLI physical fan-out node session; provider={PROVIDER}; model={MODEL}; "
            f"sandbox=read-only; no tools requested; session_jsonl={session_jsonl}; output={out_file.relative_to(ROOT)}"
        )
        existing_outputs[run_id] = {
            "run_id": run_id,
            "node_id": node_id,
            "title": node["title"],
            "session_jsonl": str(session_jsonl),
            "stdout_jsonl": str(stdout_file.relative_to(ROOT)),
            "stderr_log": str(stderr_file.relative_to(ROOT)),
            "output_file": str(out_file.relative_to(ROOT)),
            "usage": usage,
            "token_event_count": usage_payload["token_event_count"],
            "output_text": output_text,
        }
        outputs["runs"] = [existing_outputs[key] for key in sorted(existing_outputs)]
        _write_json(OUTPUTS_PATH, outputs)
        _save_manifest(manifest)
        print(f"completed {run_id}: total={usage.get('total_tokens')}", flush=True)
        completed += 1


def _pct(value: float | None) -> float | None:
    return None if value is None else round(value * 100, 2)


def _ratio(numerator: int | None, denominator: int | None) -> float | None:
    if numerator is None or denominator in (None, 0):
        return None
    return numerator / denominator


def report() -> None:
    manifest = _load_json(MANIFEST_PATH)
    nodes = manifest["nodes"]
    missing = [node["run_id"] for node in nodes if not node.get("usage", {}).get("total_tokens") or node.get("quality") != "passed"]
    totals = [int(node["usage"]["total_tokens"]) for node in nodes if node.get("usage", {}).get("total_tokens")]
    prompt_total = sum(int(node["usage"].get("prompt_tokens") or 0) for node in nodes)
    completion_total = sum(int(node["usage"].get("completion_tokens") or 0) for node in nodes)
    cached_total = sum(int(node["usage"].get("cached_input_tokens") or 0) for node in nodes)
    reasoning_total = sum(int(node["usage"].get("reasoning_output_tokens") or 0) for node in nodes)
    physical_total = sum(totals)
    baselines = manifest["baselines"]
    single = int(baselines["ait_dag_single_packet"])
    git = int(baselines["git_linear"])
    ait_linear = int(baselines["ait_linear"])
    comparisons = {
        "vs_git_linear": {
            "baseline_total_tokens": git,
            "physical_fanout_total_tokens": physical_total,
            "saving_ratio": None if missing else 1 - physical_total / git,
            "saving_percent": None if missing else round((1 - physical_total / git) * 100, 2),
            "cost_multiplier": None if missing else round(physical_total / git, 3),
        },
        "vs_ait_linear": {
            "baseline_total_tokens": ait_linear,
            "physical_fanout_total_tokens": physical_total,
            "saving_ratio": None if missing else 1 - physical_total / ait_linear,
            "saving_percent": None if missing else round((1 - physical_total / ait_linear) * 100, 2),
            "cost_multiplier": None if missing else round(physical_total / ait_linear, 3),
        },
        "vs_ait_dag_single_packet": {
            "baseline_total_tokens": single,
            "physical_fanout_total_tokens": physical_total,
            "overhead_ratio": None if missing else physical_total / single - 1,
            "overhead_percent": None if missing else round((physical_total / single - 1) * 100, 2),
            "cost_multiplier": None if missing else round(physical_total / single, 3),
        },
    }
    payload = {
        "benchmark_id": manifest["benchmark_id"],
        "description": manifest["description"],
        "measurement_type": manifest["measurement_type"],
        "provider": manifest["provider"],
        "model": manifest["model"],
        "manifest_path": str(MANIFEST_PATH.relative_to(ROOT)),
        "outputs_path": str(OUTPUTS_PATH.relative_to(ROOT)),
        "source_graph": manifest["source_graph"],
        "prompt_packets": manifest["prompt_packets"],
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "claim_ready": False,
        "verdict": "physical_overhead_measured" if not missing else "pending",
        "claim_caveat": manifest["claim_boundary"],
        "summary": {
            "node_count": len(nodes),
            "passed_node_count": len(nodes) - len(missing),
            "missing_node_count": len(missing),
            "missing_runs": missing,
            "prompt_tokens": prompt_total,
            "completion_tokens": completion_total,
            "total_tokens": physical_total,
            "cached_input_tokens": cached_total,
            "reasoning_output_tokens": reasoning_total,
            "per_node_min_total_tokens": min(totals) if totals else None,
            "per_node_median_total_tokens": int(statistics.median(totals)) if totals else None,
            "per_node_mean_total_tokens": round(statistics.mean(totals), 2) if totals else None,
            "per_node_max_total_tokens": max(totals) if totals else None,
        },
        "baselines": baselines,
        "comparisons": comparisons,
        "nodes": nodes,
    }
    _write_json(REPORT_JSON, payload)
    REPORT_MD.write_text(_render_markdown(payload), encoding="utf-8")


def _fmt_int(value: Any) -> str:
    if value is None:
        return "n/a"
    return f"{int(value):,}"


def _fmt_pct(value: Any) -> str:
    if value is None:
        return "n/a"
    return f"{float(value):.2f}%"


def _render_markdown(payload: dict[str, Any]) -> str:
    s = payload["summary"]
    c = payload["comparisons"]
    compact_tokens = payload["baselines"]["ait_dag_single_packet"]
    git_tokens = c["vs_git_linear"]["baseline_total_tokens"]
    compact_savings = ((git_tokens - compact_tokens) / git_tokens * 100.0) if git_tokens else None
    lines = [
        "# M3 20-Node Physical Fan-Out Measured Report",
        "",
        "Authority: command layer under [../plan.md](../plan.md) and the legal-layer governance documents.",
        "Status: generated provider-measured physical fan-out report; not standalone product-claim evidence.",
        "Scope: sums provider-reported token usage from one fresh Codex session per M3 20-node DAG node and compares it with prior single prompt-packet baselines.",
        "",
        "## Verdict",
        "",
        f"- Evidence type: `provider-measured physical fan-out`",
        f"- Nodes passed: `{s['passed_node_count']}` / `{s['node_count']}`",
        f"- Physical fan-out total tokens: `{_fmt_int(s['total_tokens'])}`",
        f"- Prior single-packet `ait_dag` tokens: `{_fmt_int(payload['baselines']['ait_dag_single_packet'])}`",
        f"- Physical vs single-packet multiplier: `{c['vs_ait_dag_single_packet']['cost_multiplier']}`x",
        f"- Physical vs `git_linear` multiplier: `{c['vs_git_linear']['cost_multiplier']}`x",
        f"- Claim ready: `false`",
        f"- Caveat: {payload['claim_caveat']}",
        "",
        "## Plain-language takeaway",
        "",
        "This result does **not** mean task graphs inherently cost more tokens. It means the naive execution model of running each DAG node as its own fresh provider session has high repeated setup/context overhead.",
        "",
        "| Execution model | Tokens | Reading |",
        "| --- | ---: | --- |",
        f"| Compact single-packet `ait_dag` | {_fmt_int(compact_tokens)} | Scoped DAG context is token-efficient; {_fmt_pct(compact_savings)} savings vs `git_linear` in the M3 single-packet proof point. |",
        f"| Physical 20-node fresh-session fan-out | {_fmt_int(s['total_tokens'])} | Execution-level overhead proof point; {c['vs_ait_dag_single_packet']['cost_multiplier']}x the compact `ait_dag` packet and {c['vs_git_linear']['cost_multiplier']}x the `git_linear` packet. |",
        "",
        "Operational rule: use task graphs as a compact coordinator/context layer, then batch or coalesce nodes before dispatch. Avoid one fresh provider session per small node unless wall-clock parallelism is worth the extra token cost.",
        "",
        "## Token totals",
        "",
        "| Metric | Tokens |",
        "| --- | ---: |",
        f"| Prompt/input | {_fmt_int(s['prompt_tokens'])} |",
        f"| Completion/output | {_fmt_int(s['completion_tokens'])} |",
        f"| Total | {_fmt_int(s['total_tokens'])} |",
        f"| Cached input | {_fmt_int(s['cached_input_tokens'])} |",
        f"| Reasoning output | {_fmt_int(s['reasoning_output_tokens'])} |",
        "",
        "## Comparisons",
        "",
        "| Baseline | Baseline tokens | Physical fan-out tokens | Savings | Cost multiplier |",
        "| --- | ---: | ---: | ---: | ---: |",
        f"| `git_linear` single packet | {_fmt_int(c['vs_git_linear']['baseline_total_tokens'])} | {_fmt_int(s['total_tokens'])} | {_fmt_pct(c['vs_git_linear']['saving_percent'])} | {c['vs_git_linear']['cost_multiplier']}x |",
        f"| `ait_linear` single packet | {_fmt_int(c['vs_ait_linear']['baseline_total_tokens'])} | {_fmt_int(s['total_tokens'])} | {_fmt_pct(c['vs_ait_linear']['saving_percent'])} | {c['vs_ait_linear']['cost_multiplier']}x |",
        f"| `ait_dag` single packet | {_fmt_int(c['vs_ait_dag_single_packet']['baseline_total_tokens'])} | {_fmt_int(s['total_tokens'])} | overhead {_fmt_pct(c['vs_ait_dag_single_packet']['overhead_percent'])} | {c['vs_ait_dag_single_packet']['cost_multiplier']}x |",
        "",
        "## Per-node distribution",
        "",
        "| Metric | Tokens |",
        "| --- | ---: |",
        f"| Min | {_fmt_int(s['per_node_min_total_tokens'])} |",
        f"| Median | {_fmt_int(s['per_node_median_total_tokens'])} |",
        f"| Mean | {_fmt_int(round(s['per_node_mean_total_tokens'])) if s['per_node_mean_total_tokens'] is not None else 'n/a'} |",
        f"| Max | {_fmt_int(s['per_node_max_total_tokens'])} |",
        "",
        "## Node runs",
        "",
        "| Node | Title | Total tokens | Quality |",
        "| --- | --- | ---: | --- |",
    ]
    for node in payload["nodes"]:
        lines.append(f"| `{node['node_id']}` | {node['title']} | {_fmt_int(node['usage'].get('total_tokens'))} | `{node.get('quality')}` |")
    lines.extend([
        "",
        "## Interpretation",
        "",
        "This report measures execution-level fan-out overhead, not the compact prompt-packet design alone. The physical fan-out total should be compared with both the prior `git_linear` baseline and the prior single-packet `ait_dag` run. Negative savings means the physical fan-out consumed more total provider tokens than that baseline.",
        "",
        "The result is one M3 workload proof point and must not replace the broader provider-measured product claim gate.",
        "",
    ])
    return "\n".join(lines)


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("command", choices=["prepare", "run", "report", "all"])
    parser.add_argument("--limit", type=int, default=None, help="Run at most N missing node sessions.")
    args = parser.parse_args()
    if args.command == "prepare":
        prepare()
    elif args.command == "run":
        run(limit=args.limit)
    elif args.command == "report":
        report()
    elif args.command == "all":
        prepare()
        run(limit=args.limit)
        report()


if __name__ == "__main__":
    main()
