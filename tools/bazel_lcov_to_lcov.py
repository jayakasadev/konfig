#!/usr/bin/env python3
"""Convert Bazel-generated LCOV to standard LCOV that cargo-crap can parse.

Bazel emits:
  FNL:<id>,<line>          — function id → source line mapping
  FNA:<id>,<count>,<name>  — function id → hit count + mangled name

Standard LCOV expects:
  FN:<line>,<name>
  FNDA:<count>,<name>
  FNF:<total>
  FNH:<hit>

Usage: python3 tools/bazel_lcov_to_lcov.py <input.info> <output.info>
"""
import sys


def flush_fns(out, fn_entries):
    for line, count, name in fn_entries:
        out.write(f"FN:{line},{name}\n")
    for line, count, name in fn_entries:
        out.write(f"FNDA:{count},{name}\n")
    fn_entries.clear()


def convert(src, dst):
    fn_lines = {}
    fn_entries = []
    with open(src) as inp, open(dst, "w") as out:
        for raw_line in inp:
            rec = raw_line.rstrip("\n")
            if rec.startswith("FNL:"):
                idx, lineno = rec[4:].split(",", 1)
                fn_lines[int(idx)] = lineno
            elif rec.startswith("FNA:"):
                idx_s, rest = rec[4:].split(",", 1)
                count, name = rest.split(",", 1)
                fn_entries.append((fn_lines[int(idx_s)], count, name))
            elif rec.startswith("FNF:") or rec.startswith("FNH:"):
                if fn_entries:
                    flush_fns(out, fn_entries)
                out.write(raw_line)
            elif rec == "end_of_record":
                fn_lines.clear()
                out.write(raw_line)
            else:
                out.write(raw_line)


if __name__ == "__main__":
    if len(sys.argv) != 3:
        print("Usage: {} <input.info> <output.info>".format(sys.argv[0]), file=sys.stderr)
        sys.exit(1)
    convert(sys.argv[1], sys.argv[2])
