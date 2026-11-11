#!/usr/bin/env python3
"""Summarises the tiles drawn during an ISOMAR_BENCH run from an ISOMAR_DEBUG_TIMING log
(scripts/bench.sh --timing): how many were drawn, how many distinct, how many of those had been drawn
before the bench began (drawn again for nothing, if no data arrived), the time per layer, and why tiles
were drawn again (the `redrawn` lines: the layers whose revision moved).

Usage: bench-layers.py <log>
"""
import collections
import datetime
import re
import sys


def ts(line):
    return datetime.datetime.strptime(line[:23], "%Y-%m-%d %H:%M:%S.%f")


def main(path):
    lines = open(path, errors="replace").read().splitlines()
    bench = [l for l in lines if re.search(r"isomar: bench (pan|zoom) over", l)]
    if not bench:
        print("  no bench line")
        return
    dur = float(re.search(r"over ([\d.]+) s", bench[-1]).group(1))
    end = ts(bench[-1])
    start = end - datetime.timedelta(seconds=dur)
    layers = collections.Counter()
    during, before = collections.Counter(), collections.Counter()
    total = 0
    causes = collections.Counter()
    for l in lines:
        m = re.search(r"\] isomar: tile (\d+/-?\d+/-?\d+) (\d+) ms \[(.*)\]", l)
        if m:
            key = m.group(1)
            if ts(l) < start:
                before[key] += 1
                continue
            during[key] += 1
            total += int(m.group(2))
            for part in m.group(3).split(", "):
                if part:
                    k, v = part.rsplit(" ", 1)
                    layers[k] += int(v)
            continue
        m = re.search(r"\] isomar: redrawn \S+ for (.*)", l)
        if m and ts(l) >= start:
            causes[m.group(1)] += 1
    n = sum(during.values())
    again = sum(1 for k in during if k in before)
    print(f"  {n} tiles drawn ({len(during)} distinct, {again} of them drawn before the bench), {total} ms, {total / max(n, 1):.0f} ms a tile")
    print(f"  before the bench: {sum(before.values())} drawn, {len(before)} distinct")
    print("  layers: " + ", ".join(f"{k} {v}" for k, v in layers.most_common()))
    if causes:
        print("  drawn again for: " + ", ".join(f"{k} x{v}" for k, v in causes.most_common(8)))


if __name__ == "__main__":
    main(sys.argv[1])
