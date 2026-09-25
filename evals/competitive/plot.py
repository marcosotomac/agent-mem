#!/usr/bin/env python3
"""Render the audited retrieval benchmark as a static, shareable figure."""

from __future__ import annotations

import json
from pathlib import Path

import matplotlib.pyplot as plt
import numpy as np


ROOT = Path(__file__).resolve().parent
rows = json.loads((ROOT / "results" / "summary.json").read_text())
by_key = {(row["name"], row["profile"]): row for row in rows}
order = ["agent-mem", "engram", "mem0", "tencentdb", "projectmem"]
labels = ["agent-mem", "Engram", "Mem0", "TencentDB L0", "ProjectMem"]
y = np.arange(len(order))
natural = [by_key[(name, "natural")] for name in order]
keyword = [by_key[(name, "keyword")] for name in order]
colors = ["#1261A0", "#7290A7", "#7F97A5", "#769B92", "#9CA4AD"]

plt.rcParams.update({"font.family": "DejaVu Sans", "font.size": 10})
fig, axes = plt.subplots(1, 2, figsize=(12.5, 5.4),
                         gridspec_kw={"width_ratios": [1.2, 1.0]})
fig.patch.set_facecolor("#FFFFFF")
for ax in axes:
    ax.set_facecolor("#FAFBFC")
    ax.spines[["top", "right"]].set_visible(False)
    ax.set_yticks(y, labels)
    ax.invert_yaxis()
    ax.grid(axis="x", color="#E6EAEE", linewidth=.8)
    ax.set_axisbelow(True)

quality = axes[0]
quality.barh(y - .17, [row["recall_at_5"] for row in natural], height=.30,
             color=colors, label="Consulta natural")
quality.barh(y + .17, [row["recall_at_5"] for row in keyword], height=.30,
             color=colors, alpha=.38, label="Palabra clave")
quality.set_xlim(0, 1.13)
quality.set_xticks([0, .25, .5, .75, 1], ["0", "25", "50", "75", "100 %"])
quality.set_xlabel("Recall@5")
quality.set_title("Recuperación de la respuesta correcta", weight="bold", pad=13)
fig.legend(*quality.get_legend_handles_labels(), loc="upper left",
           bbox_to_anchor=(.055, .948), ncol=2, frameon=False, fontsize=9)
for idx, row in enumerate(natural):
    quality.text(row["recall_at_5"] + .018, idx - .17,
                 f"{row['recall_at_5']:.0%}", va="center", fontsize=9)

latency = axes[1]
values = [row["query_ms_p50"] for row in natural]
latency.barh(y, values, height=.55, color=colors)
latency.set_xscale("log")
latency.set_xlim(.07, max(values) * 2.4)
latency.set_xlabel("Latencia p50 por consulta (ms, escala logarítmica)")
latency.set_title("Tiempo de respuesta del sistema", weight="bold", pad=13)
for idx, value in enumerate(values):
    latency.text(value * 1.12, idx, f"{value:.2f} ms", va="center", fontsize=9)

fig.suptitle("Memoria de agentes · 210 registros, 20 consultas naturales",
             x=.06, ha="left", fontsize=15, weight="bold", y=.99)
fig.text(.06, .012, "Docker Linux/ARM64 · 5 pasadas · Mem0 incluye embeddings remotos · "
         "TencentDB mide conversación L0 por HTTP; los otros tres usan MCP stdio",
         fontsize=8, color="#586672")
fig.tight_layout(rect=(0, .045, 1, .94), w_pad=2.5)
svg_path = ROOT / "metrics.svg"
fig.savefig(svg_path, bbox_inches="tight")
svg_path.write_text("\n".join(line.rstrip() for line in svg_path.read_text().splitlines()) + "\n")
fig.savefig(ROOT / "metrics.png", dpi=180, bbox_inches="tight")
