#!/usr/bin/env python3
"""Reads the eu-stack samples scripts/profile.sh wrote and prints the functions busy threads were in:
self (the top frame) and inclusive (anywhere on the stack), as a share of the busy samples.

Usage: profile-report.py <samples.txt> [filter] [rows]
"""
import collections
import re
import sys

# Top frames of a thread that is waiting, not working.
IDLE = ("futex_wait", "__futex_abstimed_wait", "syscall", "poll", "epoll_wait", "nanosleep", "ppoll",
        "pthread_cond_wait", "clock_nanosleep", "__GI___libc_read", "rayon_core::sleep", "g_main_context_iterate")


def parse(path):
    samples, cur = [], None
    for line in open(path, errors="replace"):
        line = line.rstrip("\n")
        if re.match(r"TID (\d+):", line):
            cur = []
            samples.append(cur)
            continue
        m = re.match(r"#\d+\s+0x[0-9a-f]+\s*(?:- \d+\s*)?(.*)", line)
        if m and cur is not None:
            cur.append(re.sub(r"::h[0-9a-f]{16}$", "", m.group(1).strip()))
    return samples


def main(path, filt="", rows=40):
    samples = parse(path)
    busy = [s for s in samples if s and not any(k in s[0] for k in IDLE)]
    if filt:
        busy = [s for s in busy if any(filt in f for f in s)]
    tot = len(busy) or 1
    print(f"{len(samples)} thread samples, {len(busy)} busy{' with ' + filt if filt else ''}")
    own, incl = collections.Counter(), collections.Counter()
    for frames in busy:
        own[frames[0]] += 1
        for f in set(frames):
            incl[f] += 1
    print("--- self")
    for f, c in own.most_common(rows):
        print(f"{100 * c / tot:5.1f}% {c:5d} {f[:150]}")
    print("--- inclusive")
    for f, c in incl.most_common(rows * 2):
        print(f"{100 * c / tot:5.1f}% {c:5d} {f[:150]}")


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2] if len(sys.argv) > 2 else "", int(sys.argv[3]) if len(sys.argv) > 3 else 40)
