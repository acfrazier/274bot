#!/usr/bin/env python3
"""Print a TLC counterexample from a .out file as one line per step, in product terms.

usage: trace_summary.py <tlc.out>
"""
import re
import sys

text = open(sys.argv[1]).read()
m = re.search(r"Error: (.*)", text)
print(m.group(0) if m else "no error line")
blocks = re.split(r"\nState (\d+): ", text)
# blocks[0] = preamble; then (num, body) pairs
for i in range(1, len(blocks) - 1, 2):
    num, body = blocks[i], blocks[i + 1]
    head, _, rest = body.partition("\n")
    end = rest.find("\n\n")
    rest = rest[:end] if end >= 0 else rest
    flat = re.sub(r"\s+", " ", rest)

    def field(name, pat=r"([^,\]]+)"):
        mm = re.search(r"\b" + name + r" \|-> " + pat, flat)
        return mm.group(1).strip() if mm else "?"

    lease = re.search(r"\bL \|-> (\[.*?\]\s*\]|\"none\"|\[[^\[\]]*\](?:, )?)", flat)
    lk = re.search(r"\bL \|-> \[ ?kind \|-> \"(.)\", key \|-> <<[^>]*>>, txn \|-> (\d+), ph \|-> \"(\w+)\", own \|-> \[k \|-> \"(.)\", v \|-> (\d+)\], st \|-> \[k \|-> \"(.)\", v \|-> (\d+)\], since \|-> (\d+)", flat)
    if lk:
        ls = "L=%s/%s txn%s own=%s%s st=%s%s since=%s" % lk.groups()[0:1] + ("",) if False else \
             "L=%s/%s txn%s own=%s%s st=%s%s since=%s" % (lk.group(1), lk.group(3), lk.group(2), lk.group(4), lk.group(5), lk.group(6), lk.group(7), lk.group(8))
    else:
        ls = "L=none"
    hw = re.search(r"\bhw \|-> \[ ?key \|-> <<\"(.)\", (\d+)>>, txn \|-> (\d+), rel \|-> (\w+)", flat)
    hs = "hw=none" if (not hw or hw.group(1) == "-") else "hw=%s%s%s" % (hw.group(1), hw.group(2), " REL" if hw.group(4) == "TRUE" else "")
    act = field("act", r"\"([^\"]*)\"")
    print("%2s %-26s pc=%-6s tick=%s wall=%s act=%s clk=%s alive=%s want=%s held=%s hung=%s modal=%s tkE=%s dirty=%s bnd=%s | %s | %s" % (
        num, act, field("pc", r"\"(\w+)\""), field("tick"), field("wall"), field("actV"), field("clkOpen"),
        field("alive"), field("want"), field("held"), field("hung"), field("modal"), field("tickEdge"),
        field("dirty"), field("bnd"), ls, hs))
