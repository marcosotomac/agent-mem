import unittest

import run_matrix


class BuildJobTest(unittest.TestCase):
    def test_memory_server_uses_harbor_repository_path(self) -> None:
        matrix = run_matrix.load_matrix()

        job = run_matrix.build_job(matrix, "agent-mem", matrix["models"][0])

        self.assertEqual(
            job["agents"][0]["mcp_servers"][0]["args"][2],
            "/testbed",
        )

    def test_agent_mem_mounts_linux_build_cache(self) -> None:
        matrix = run_matrix.load_matrix()

        job = run_matrix.build_job(matrix, "agent-mem", matrix["models"][0])

        mount = next(
            item for item in job["environment"]["mounts"]
            if item["target"] == "/opt/agent-mem-bin"
        )
        self.assertEqual(
            mount["source"],
            str(run_matrix.LINUX_AGENT_MEM_TARGET / "release"),
        )


if __name__ == "__main__":
    unittest.main()
