import json
from pathlib import Path
from tempfile import TemporaryDirectory
import unittest

import report


class MemoryToolEvidenceTest(unittest.TestCase):
    def write_trajectory(self, root: Path, names: list[str], content: str = "retrieved memory") -> None:
        agent = root / "agent"
        agent.mkdir()
        trajectory = {
            "steps": [{
                "tool_calls": [
                    {"tool_call_id": f"call-{index}", "function_name": name}
                    for index, name in enumerate(names)
                ],
                "observation": {"results": [
                    {"source_call_id": f"call-{index}", "content": content}
                    for index, _ in enumerate(names)
                ]},
            }]
        }
        (agent / "trajectory.json").write_text(json.dumps(trajectory))

    def test_agent_mem_requires_mem_find(self) -> None:
        with TemporaryDirectory() as directory:
            trial = Path(directory)
            self.write_trajectory(trial, ["mcp__agent_mem__mem_find"])
            self.assertEqual(report.memory_tool_evidence(trial, "agent-mem"), (["mem_find"], True))

    def test_projectmem_requires_both_tools(self) -> None:
        with TemporaryDirectory() as directory:
            trial = Path(directory)
            self.write_trajectory(trial, ["mcp__projectmem__get_summary"])
            self.assertEqual(report.memory_tool_evidence(trial, "projectmem"), (["get_summary"], False))

    def test_missing_trajectory_fails_closed(self) -> None:
        with TemporaryDirectory() as directory:
            self.assertEqual(report.memory_tool_evidence(Path(directory), "official-mcp-memory"), ([], False))

    def test_failed_memory_call_does_not_count(self) -> None:
        with TemporaryDirectory() as directory:
            trial = Path(directory)
            self.write_trajectory(trial, ["mem_find"], "Error: Unknown project")
            self.assertEqual(report.memory_tool_evidence(trial, "agent-mem"), ([], False))


class EvidenceGateTest(unittest.TestCase):
    def test_missing_tokens_fail_gate_without_crashing(self) -> None:
        common = {
            "model": "openai/test-model",
            "task": "org__repo-1",
            "trial": "trial-1",
            "infrastructure_error": False,
            "success": True,
            "input_tokens": None,
            "output_tokens": None,
            "total_tokens": None,
            "latency_seconds": 1.0,
            "memory_tool_used": None,
        }
        rows = [
            {**common, "baseline": "native"},
            {**common, "baseline": "agent-mem", "trial": "trial-2", "memory_tool_used": True},
        ]
        matrix = {
            "baselines": ["native", "agent-mem"],
            "models": ["openai/test-model"],
            "task_globs": ["*org__repo-*"],
            "attempts": 1,
        }

        errors = report.evidence_gate(rows, matrix)

        self.assertIn("2 valid trials lack success, token, or latency evidence", errors)


if __name__ == "__main__":
    unittest.main()
