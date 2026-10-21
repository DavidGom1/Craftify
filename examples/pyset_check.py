"""Random set operations and CPython's iteration order after each, to check src/pyset.rs.

usage: python pyset_check.py ops.txt expected.txt
"""
import random
import sys

random.seed(7)
ops, out = [], []
for _case in range(400):
    s = set()
    ops.append("n")
    hi = random.choice([10, 50, 300, 5000, 200000])
    for _ in range(random.randint(1, 120)):
        k = random.randint(1, hi)
        if random.random() < 0.7 or not s:
            s.add(k)
            ops.append(f"a {k}")
        else:
            k = random.choice(sorted(s)) if random.random() < 0.8 else k
            s.discard(k)
            ops.append(f"d {k}")
        if random.random() < 0.3:
            ops.append("p")
            out.append(str(list(s)))
    ops.append("p")
    out.append(str(list(s)))
open(sys.argv[1], "w").write("\n".join(ops) + "\n")
open(sys.argv[2], "w").write("\n".join(out) + "\n")
