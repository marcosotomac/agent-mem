#!/usr/bin/env python3
"""Create a checksummed archive for one immutable benchmark campaign."""

from __future__ import annotations

import argparse
import hashlib
from pathlib import Path
import tarfile


EVALS = Path(__file__).resolve().parent
SECRET_MARKERS = (b"sk-proj-", b"OPENAI_API_KEY=")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--campaign", required=True)
    parser.add_argument("--output-dir", type=Path, default=EVALS / "published")
    args = parser.parse_args()

    suffix = f"__{args.campaign}"
    job_dirs = sorted(path for path in (EVALS / "jobs").iterdir() if path.is_dir() and path.name.endswith(suffix))
    report_files = [EVALS / "published" / f"{args.campaign}.json", EVALS / "published" / f"{args.campaign}.md"]
    generated = sorted((EVALS / "generated").glob(f"*{suffix}.json"))
    inputs = [EVALS / "matrix.toml", *report_files, *generated, *job_dirs]
    missing = [str(path) for path in inputs if not path.exists()]
    if missing:
        raise SystemExit(f"missing campaign evidence: {', '.join(missing)}")

    for root in inputs:
        files = [root] if root.is_file() else (path for path in root.rglob("*") if path.is_file())
        for path in files:
            data = path.read_bytes()
            if any(marker in data for marker in SECRET_MARKERS):
                raise SystemExit(f"refusing to package possible provider credential in {path}")

    args.output_dir.mkdir(parents=True, exist_ok=True)
    temporary = args.output_dir / f"{args.campaign}-evidence.tar.gz.part"
    with tarfile.open(temporary, "w:gz") as archive:
        for path in inputs:
            archive.add(path, arcname=path.relative_to(EVALS.parent), recursive=True)
    digest = hashlib.sha256(temporary.read_bytes()).hexdigest()
    final = args.output_dir / f"{args.campaign}-evidence-{digest[:16]}.tar.gz"
    temporary.replace(final)
    (args.output_dir / f"{args.campaign}-evidence.sha256").write_text(f"{digest}  {final.name}\n")
    print(final)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
