#!/usr/bin/env python3
"""Measures every crate and draws the README's badges.

    python3 tools/badges/badges.py OUT_DIR

For each crate in crates/, runs its tests under cargo-llvm-cov and records
line coverage, the number of tests, and whether they all passed. Runs spark
on the whole repository for the five Rust rules. Writes to OUT_DIR:

    summary.json          every number measured
    history-row.csv       one line for the badges branch's history.csv
    <name>.svg            one badge per number

Always exits 0 once it has measured, so failing tests still get a red badge
published; CI fails the build afterwards from summary.json.
"""

import datetime
import json
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
CRATES = sorted(path.name for path in (ROOT / "crates").iterdir() if (path / "Cargo.toml").exists())
TEST_RESULT = re.compile(r"test result: (\w+)\. (\d+) passed; (\d+) failed")
SPARK_SUMMARY = re.compile(r"(\d+) files, (\d+) problems")

GREEN, TEAL, AMBER, RED, BLUE, SLATE, RUST = "#16a34a", "#0d9488", "#d97706", "#dc2626", "#2563eb", "#1e293b", "#b7410e"


def run(command):
    """Runs `command` in the repository; returns its exit code and combined output."""
    result = subprocess.run(command, cwd=ROOT, capture_output=True, text=True)
    return result.returncode, result.stdout + result.stderr


def measure_crate(crate):
    """Line coverage, test count and pass/fail of one crate."""
    run(["cargo", "llvm-cov", "clean", "--workspace"])
    with tempfile.TemporaryDirectory() as folder:
        report = Path(folder) / "coverage.json"
        code, output = run(["cargo", "llvm-cov", "-p", crate, "--json", "--summary-only", "--output-path", str(report)])
        lines = json.loads(report.read_text())["data"][0]["totals"]["lines"] if report.exists() else {"count": 0, "covered": 0}
    results = TEST_RESULT.findall(output)
    return {
        "crate": crate,
        "passed": code == 0 and all(status == "ok" for status, _, _ in results),
        "tests": sum(int(passed) + int(failed) for _, passed, failed in results),
        "failed": sum(int(failed) for _, _, failed in results),
        "lines": lines["count"],
        "covered": lines["covered"],
        "coverage": 100.0 * lines["covered"] / lines["count"] if lines["count"] else 0.0,
    }


def measure_rules():
    """Files and problems spark finds in the repository's own Rust."""
    _, output = run(["cargo", "run", "-q", "--release", "-p", "thor-spark-safety-eval", "--", "rs", "crates"])
    found = SPARK_SUMMARY.search(output)
    return {"files": int(found.group(1)), "problems": int(found.group(2))} if found else {"files": 0, "problems": -1}


def toolchain():
    """The Rust version pinned in rust-toolchain.toml."""
    found = re.search(r'channel\s*=\s*"([^"]+)"', (ROOT / "rust-toolchain.toml").read_text())
    return found.group(1) if found else "stable"


def coverage_color(percent):
    if percent >= 90:
        return GREEN
    if percent >= 80:
        return TEAL
    if percent >= 60:
        return AMBER
    return RED


# Glyphs drawn in a 16 x 16 box, stroked in white.
ICONS = {
    "shield": "M8 1.8 2.8 3.9v3.9c0 3.2 2.2 5.6 5.2 6.4 3-.8 5.2-3.2 5.2-6.4V3.9Z M5.6 8.1l1.7 1.7 3.2-3.4",
    "flask": "M6.2 1.8h3.6 M6.8 1.8v4.3L3.2 12.6c-.5.9.1 1.9 1.1 1.9h7.4c1 0 1.6-1 1.1-1.9L9.2 6.1V1.8 M4.6 10h6.8",
    "check": "M8 1.5a6.5 6.5 0 1 0 0 13 6.5 6.5 0 0 0 0-13Z M5.2 8.2l1.9 1.9 3.8-4",
    "cross": "M8 1.5a6.5 6.5 0 1 0 0 13 6.5 6.5 0 0 0 0-13Z M5.7 5.7l4.6 4.6 M10.3 5.7l-4.6 4.6",
    "gear": "M8 5.4a2.6 2.6 0 1 0 0 5.2 2.6 2.6 0 0 0 0-5.2Z M8 1.5v2 M8 12.5v2 M1.5 8h2 M12.5 8h2 M3.4 3.4l1.4 1.4 M11.2 11.2l1.4 1.4 M3.4 12.6l1.4-1.4 M11.2 4.8l1.4-1.4",
    "scale": "M8 2v12 M4.5 14h7 M2.5 5.5h11 M2.5 5.5 1 9.5c.5 1 2.5 1 3 0Z M13.5 5.5 12 9.5c.5 1 2.5 1 3 0Z",
}

# Approximate advance widths of 12px semibold Inter, in pixels.
NARROW, WIDE = set("ijlI.,:;'|! ()[]"), set("mwMW%@")


def text_width(text):
    return sum(3.6 if c in NARROW else 10.0 if c in WIDE else 8.0 if c.isupper() else 7.0 if c.isdigit() else 6.6 for c in text)


def badge(label, value, color, icon):
    """A flat, rounded two-part badge: a dark label with an icon, and a coloured value."""
    pad, icon_size, gap, height = 10, 16, 6, 28
    label_width = round(pad + icon_size + gap + text_width(label) + pad)
    value_width = round(pad + text_width(value) + pad)
    width = label_width + value_width
    font = "font-family=\"Inter,'Segoe UI',Helvetica,Arial,sans-serif\" font-size=\"12\" font-weight=\"600\""
    return f"""<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" viewBox="0 0 {width} {height}" role="img" aria-label="{label}: {value}">
  <title>{label}: {value}</title>
  <clipPath id="r"><rect width="{width}" height="{height}" rx="8"/></clipPath>
  <g clip-path="url(#r)">
    <rect width="{label_width}" height="{height}" fill="{SLATE}"/>
    <rect x="{label_width}" width="{value_width}" height="{height}" fill="{color}"/>
  </g>
  <g transform="translate({pad} {(height - icon_size) / 2})" fill="none" stroke="#e2e8f0" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round"><path d="{ICONS[icon]}"/></g>
  <g fill="#ffffff" {font} letter-spacing=".2">
    <text x="{pad + icon_size + gap}" y="18.2" fill="#e2e8f0">{label}</text>
    <text x="{label_width + pad}" y="18.2">{value}</text>
  </g>
</svg>
"""


def write_badges(out, crates, rules):
    total_lines = sum(crate["lines"] for crate in crates)
    total_covered = sum(crate["covered"] for crate in crates)
    overall = 100.0 * total_covered / total_lines if total_lines else 0.0
    all_passed = all(crate["passed"] for crate in crates)
    tests = sum(crate["tests"] for crate in crates)
    badges = {
        "ci": badge("ci", "passing" if all_passed else "failing", GREEN if all_passed else RED, "check" if all_passed else "cross"),
        "coverage": badge("coverage", f"{overall:.0f}%", coverage_color(overall), "shield"),
        "tests": badge("tests", f"{tests} passing" if all_passed else f"{sum(c['failed'] for c in crates)} failing", BLUE if all_passed else RED, "flask"),
        "rules": badge("five rust rules", "0 problems" if rules["problems"] == 0 else f"{rules['problems']} problems",
                       GREEN if rules["problems"] == 0 else RED, "check" if rules["problems"] == 0 else "cross"),
        "rust": badge("rust", toolchain(), RUST, "gear"),
        "license": badge("license", "Apache-2.0", BLUE, "scale"),
    }
    for crate in crates:
        name = crate["crate"]
        badges[f"{name}-coverage"] = badge("coverage", f"{crate['coverage']:.0f}%", coverage_color(crate["coverage"]), "shield")
        badges[f"{name}-tests"] = badge(
            "tests",
            f"{crate['tests']} passing" if crate["passed"] else f"{crate['failed']} failing",
            BLUE if crate["passed"] else RED,
            "flask",
        )
    out.mkdir(parents=True, exist_ok=True)
    for name, svg in badges.items():
        (out / f"{name}.svg").write_text(svg)
    summary = {
        "commit": os.environ.get("GITHUB_SHA", "local")[:7],
        "measured_at": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
        "all_passed": all_passed,
        "coverage": round(overall, 2),
        "tests": tests,
        "rules": rules,
        "crates": crates,
    }
    (out / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    row = [summary["measured_at"], summary["commit"], f"{overall:.2f}", str(tests)]
    row += [f"{crate['crate']}={crate['coverage']:.2f}" for crate in crates]
    (out / "history-row.csv").write_text(",".join(row) + "\n")
    return summary


def main():
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    crates = [measure_crate(crate) for crate in CRATES]
    summary = write_badges(Path(sys.argv[1]), crates, measure_rules())
    for crate in crates:
        state = "ok" if crate["passed"] else "FAILED"
        print(f"{crate['crate']:<34} {crate['coverage']:6.2f}%  {crate['tests']:4d} tests  {state}")
    print(f"{'total':<34} {summary['coverage']:6.2f}%  {summary['tests']:4d} tests  rules: {summary['rules']}")


if __name__ == "__main__":
    main()
