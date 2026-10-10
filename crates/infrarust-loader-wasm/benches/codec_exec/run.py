#!/usr/bin/env python3
import argparse
import glob
import json
import os
import statistics
import subprocess
import sys
import time

OLD = {"STUDY_EPOCH_TICK": "50ms", "STUDY_CODEC_BUDGET": "800ms"}
NO_QUARANTINE = {"STUDY_QUARANTINE_FAULTS": "0"}

CONFIGS = {
    "default": {},
    "before": {},
    "old-budget": {**OLD, **NO_QUARANTINE},
    "no-quarantine": {**NO_QUARANTINE},
    "budget-2ms": {"STUDY_CODEC_BUDGET": "2ms"},
    "budget-10ms": {"STUDY_CODEC_BUDGET": "10ms"},
}

PROFILES = {
    "w2pinned": {"env": {"STUDY_WORKERS": "2", "STUDY_SPINNERS": "4"}, "cpus": "2,3"},
    "w2": {"env": {"STUDY_WORKERS": "2", "STUDY_SPINNERS": "4"}, "cpus": None},
    "wdefault": {"env": {"STUDY_WORKERS": "0", "STUDY_SPINNERS": "10"}, "cpus": None},
    "wdefault32": {"env": {"STUDY_WORKERS": "0", "STUDY_SPINNERS": "32"}, "cpus": None},
    "sparse": {
        "env": {
            "STUDY_WORKERS": "0",
            "STUDY_SPINNERS": "10",
            "STUDY_VICTIMS": "20",
            "STUDY_RATE_HZ": "10",
            "STUDY_ATTACK_TRIGGER": "io",
            "STUDY_ATTACK_PERIOD_MS": "3000",
            "STUDY_ATTACK_MS": "10000",
        },
        "cpus": None,
    },
    "sparse-w2pinned": {
        "env": {
            "STUDY_WORKERS": "2",
            "STUDY_SPINNERS": "4",
            "STUDY_VICTIMS": "20",
            "STUDY_RATE_HZ": "10",
            "STUDY_ATTACK_TRIGGER": "io",
            "STUDY_ATTACK_PERIOD_MS": "3000",
            "STUDY_ATTACK_MS": "10000",
        },
        "cpus": "2,3",
    },
}


def find_binary(root):
    candidates = [
        path
        for path in glob.glob(os.path.join(root, "target", "release", "deps", "codec_exec-*"))
        if os.access(path, os.X_OK) and not path.endswith(".d")
    ]
    if not candidates:
        sys.exit("build first: cargo bench -p infrarust-loader-wasm --features wasm --bench codec_exec --no-run")
    return max(candidates, key=os.path.getmtime)


def run_once(binary, scenario, config, profile, extra_env):
    if config == "before":
        binary = os.environ.get("STUDY_BINARY_BEFORE") or sys.exit("config before needs STUDY_BINARY_BEFORE")
    env = dict(os.environ)
    env.update(CONFIGS[config])
    if profile:
        env.update(PROFILES[profile]["env"])
    env.update(extra_env)
    command = [binary, scenario]
    cpus = PROFILES[profile]["cpus"] if profile else None
    if cpus:
        command = ["taskset", "-c", cpus] + command
    started = time.time()
    with open("/proc/loadavg") as handle:
        load = float(handle.read().split()[0])
    result = subprocess.run(command, env=env, capture_output=True, text=True, timeout=600)
    metrics = {"host.loadavg1": load}
    for line in result.stdout.splitlines():
        if line.startswith("{"):
            record = json.loads(line)
            metrics[record["metric"]] = record["value"]
    if result.returncode != 0:
        sys.stderr.write(result.stderr[-4000:])
        raise SystemExit(f"{scenario} {config} {profile} exited with {result.returncode}")
    return {"metrics": metrics, "seconds": round(time.time() - started, 1)}


def summarize(values):
    values = [v for v in values if v == v]
    if not values:
        return "n/a"
    median = statistics.median(values)
    return f"{fmt(median)} [{fmt(min(values))}–{fmt(max(values))}]"


def fmt(value):
    if value >= 100:
        return f"{value:.0f}"
    if value >= 10:
        return f"{value:.1f}"
    return f"{value:.2f}"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("scenario", choices=["hot", "rr", "create", "load", "isolation", "idle", "sizes"])
    parser.add_argument("--configs", default="default")
    parser.add_argument("--profile", default=None, choices=[None, *PROFILES.keys()])
    parser.add_argument("--runs", type=int, default=3)
    parser.add_argument("--env", action="append", default=[])
    parser.add_argument("--out", default=None)
    parser.add_argument("--root", default=os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "..", "..", "..")))
    args = parser.parse_args()
    binary = os.environ.get("STUDY_BINARY") or find_binary(os.environ.get("CARGO_TARGET_DIR", os.path.join(args.root, "target")).rsplit("/target", 1)[0])
    extra_env = dict(item.split("=", 1) for item in args.env)
    results = {}
    for run in range(args.runs):
        for config in args.configs.split(","):
            outcome = run_once(binary, args.scenario, config, args.profile, extra_env)
            results.setdefault(config, []).append(outcome["metrics"])
            print(f"# run {run + 1} {config} {args.profile or ''} {outcome['seconds']}s", file=sys.stderr, flush=True)
    if args.out:
        with open(args.out, "w") as handle:
            json.dump({"scenario": args.scenario, "profile": args.profile, "env": extra_env, "results": results}, handle, indent=1)
    metrics = []
    for runs in results.values():
        for run in runs:
            for metric in run:
                if metric not in metrics:
                    metrics.append(metric)
    print(f"\n{args.scenario} {args.profile or ''} {extra_env or ''} runs={args.runs}: median [min–max]\n")
    print("| metric | " + " | ".join(results) + " |")
    print("|---|" + "---|" * len(results))
    for metric in metrics:
        cells = [summarize([run.get(metric, float("nan")) for run in runs]) for runs in results.values()]
        print(f"| {metric} | " + " | ".join(cells) + " |")


if __name__ == "__main__":
    main()
