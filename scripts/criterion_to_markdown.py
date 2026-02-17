# just for transparency, this script is 99.9% AI generated I just added #type:ignore so my linter wouldn't cry

"""Read the latest criterion results and output markdown tables for the README.

Usage: python scripts/criterion_to_markdown.py [target/criterion]
"""

import json
import sys
from pathlib import Path


def read_estimate(path: Path) -> float | None:
    """Return the slope point_estimate (ns) from a criterion estimates.json."""
    estimates = path / "new" / "estimates.json"
    if not estimates.exists():
        return None
    data = json.loads(estimates.read_text())
    return data["slope"]["point_estimate"]


def fmt_time(ns: float) -> str:
    if ns >= 1_000_000:
        return f"{ns / 1_000_000:.1f}ms"
    if ns >= 1_000:
        return f"{ns / 1_000:.1f}\u00b5s"
    return f"{ns:.1f}ns"


def fmt_ratio(a: float, b: float) -> str:
    r = a / b
    if r >= 10:
        return f"**{r:.0f}x**"
    return f"**{r:.1f}x**"


def parse_line_table(criterion: Path) -> str:
    group = criterion / "parse_line"
    lines = [
        ("data field", "data_field"),
        ("comment", "comment"),
        ("event field", "event_field"),
        ("id field", "id_field"),
        ("empty line", "empty_line"),
        ("no value", "no_value"),
        ("no space", "no_space"),
        ("big 1024-byte line", "big_data_line"),
    ]

    rows: list[str] = []
    rows.append("| Line type | eventsource-stream | sseer |")
    rows.append("|---|---|---|")

    for display, key in lines:
        sseer = read_estimate(group / "sseer" / key)
        es = read_estimate(group / "eventsource_stream" / key)
        if sseer is None or es is None:
            continue
        rows.append(
            f"| {display} | {fmt_time(es)} | {fmt_time(sseer)} ({fmt_ratio(es, sseer)}) |"
        )

    return "\n".join(rows)


def event_stream_table(criterion: Path) -> str:
    group = criterion / "event_stream"
    workloads = [
        ("mixed", "unaligned", "mixed_unaligned"),
        ("mixed", "aligned", "mixed_aligned"),
        ("ai_stream", "unaligned", "ai_stream_unaligned"),
        ("ai_stream", "aligned", "ai_stream_aligned"),
        ("evenish_distribution", "unaligned", "evenish_distribution_unaligned"),
    ]

    rows: list[str] = []
    rows.append("| Workload | Chunking | eventsource-stream | sseer (generic) | sseer (bytes) |")
    rows.append("|---|---|---|---|---|")

    for display, chunking, key in workloads:
        es = read_estimate(group / "eventsource_stream" / key)
        generic = read_estimate(group / "sseer" / key)
        bytes_ = read_estimate(group / "sseer_bytes_only" / key)

        if es is None or generic is None or bytes_ is None:
            continue

        rows.append(
            f"| {display} | {chunking} "
            f"| {fmt_time(es)} "
            f"| {fmt_time(generic)} ({fmt_ratio(es, generic)}) "
            f"| {fmt_time(bytes_)} ({fmt_ratio(es, bytes_)}) |"
        )

    return "\n".join(rows)


def main() -> None:
    sys.stdout.reconfigure(encoding="utf-8") #type:ignore

    criterion = Path(sys.argv[1]) if len(sys.argv) > 1 else Path("target/criterion")

    if not criterion.exists():
        print(f"error: {criterion} not found", file=sys.stderr)
        sys.exit(1)

    print("### parse_line")
    print()
    print(parse_line_table(criterion))
    print()
    print("### event_stream")
    print()
    print(event_stream_table(criterion))


if __name__ == "__main__":
    main()
