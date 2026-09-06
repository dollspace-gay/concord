#!/usr/bin/env python3
"""Isolated mutation tests for check-actor-service-boundaries.py."""

from __future__ import annotations

import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
CHECKER = ROOT / "scripts/check-actor-service-boundaries.py"


class ActorServiceBoundaryTests(unittest.TestCase):
    maxDiff = None

    def run_checker(self, mutate=None) -> subprocess.CompletedProcess[str]:
        with tempfile.TemporaryDirectory(prefix="concord-boundary-") as directory:
            fixture = Path(directory)
            source = fixture / "concord/server/src"
            source.parent.mkdir(parents=True)
            shutil.copytree(ROOT / "concord/server/src", source)
            if mutate is not None:
                mutate(source)
            return subprocess.run(
                [str(CHECKER), "--root", str(fixture)],
                text=True,
                capture_output=True,
                check=False,
            )

    def assert_rejected(self, mutate, expected: str) -> None:
        result = self.run_checker(mutate)
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn(expected, result.stderr)

    def test_current_tree_passes(self) -> None:
        result = self.run_checker()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("production HTTP handlers/helpers", result.stdout)
        self.assertIn("all 124 WebSocket operations", result.stdout)

    def test_rejects_direct_http_repository_access(self) -> None:
        def mutate(source: Path) -> None:
            path = source / "web/rest_api.rs"
            text = path.read_text()
            marker = "pub async fn get_me("
            start = text.index(marker)
            opening = text.index("{", start)
            text = text[: opening + 1] + "\n    let _repository = &state.db;" + text[opening + 1 :]
            path.write_text(text)

        self.assert_rejected(mutate, "HTTP adapter get_me")

    def test_rejects_aliased_http_query_module(self) -> None:
        def mutate(source: Path) -> None:
            path = source / "web/rest_api.rs"
            text = "use crate::db::queries::users as user_repository;\n" + path.read_text()
            marker = "pub async fn get_user_profile("
            start = text.index(marker)
            opening = text.index("{", start)
            text = (
                text[: opening + 1]
                + "\n    let _repository_call = user_repository::get_user;"
                + text[opening + 1 :]
            )
            path.write_text(text)

        self.assert_rejected(mutate, "repository alias user_repository")

    def test_rejects_directly_imported_sqlx_function_alias(self) -> None:
        def mutate(source: Path) -> None:
            path = source / "web/rest_api.rs"
            text = "use sqlx::query as q;\n" + path.read_text()
            marker = "pub async fn get_me("
            start = text.index(marker)
            opening = text.index("{", start)
            text = text[: opening + 1] + '\n    let _query = q("SELECT 1");' + text[opening + 1 :]
            path.write_text(text)

        self.assert_rejected(mutate, "repository symbol q")

    def test_rejects_directly_imported_query_function_item(self) -> None:
        def mutate(source: Path) -> None:
            path = source / "web/rest_api.rs"
            text = (
                "use crate::db::queries::users::get_user as lookup;\n"
                + path.read_text()
            )
            marker = "pub async fn get_user_profile("
            start = text.index(marker)
            opening = text.index("{", start)
            text = text[: opening + 1] + "\n    let _repository = lookup;" + text[opening + 1 :]
            path.write_text(text)

        self.assert_rejected(mutate, "repository symbol lookup")

    def test_rejects_braced_direct_query_function_alias(self) -> None:
        def mutate(source: Path) -> None:
            path = source / "web/rest_api.rs"
            text = (
                "use crate::db::queries::users::{get_user as lookup};\n"
                + path.read_text()
            )
            marker = "pub async fn get_user_profile("
            start = text.index(marker)
            opening = text.index("{", start)
            text = text[: opening + 1] + "\n    let _repository = lookup;" + text[opening + 1 :]
            path.write_text(text)

        self.assert_rejected(mutate, "repository symbol lookup")

    def test_rejects_nested_database_module_alias(self) -> None:
        def mutate(source: Path) -> None:
            path = source / "web/rest_api.rs"
            text = "use crate::db::{queries as repo};\n" + path.read_text()
            marker = "pub async fn get_user_profile("
            start = text.index(marker)
            opening = text.index("{", start)
            text = text[: opening + 1] + "\n    let _repository = repo::users::get_user;" + text[opening + 1 :]
            path.write_text(text)

        self.assert_rejected(mutate, "repository alias repo")

    def test_rejects_plain_database_queries_import_in_late_helper(self) -> None:
        def mutate(source: Path) -> None:
            path = source / "web/rest_api.rs"
            text = "use crate::db::queries;\n" + path.read_text()
            text += (
                "\npub fn late_parent_probe() {\n"
                "    let _repository_call = queries::users::get_user;\n"
                "}\n"
            )
            path.write_text(text)

        self.assert_rejected(mutate, "repository alias queries")

    def test_rejects_database_root_alias(self) -> None:
        def mutate(source: Path) -> None:
            path = source / "web/rest_api.rs"
            text = "use crate::db as repository;\n" + path.read_text()
            marker = "pub async fn get_me("
            start = text.index(marker)
            opening = text.index("{", start)
            text = (
                text[: opening + 1]
                + "\n    let _repository_call = repository::queries::users::get_user;"
                + text[opening + 1 :]
            )
            path.write_text(text)

        self.assert_rejected(mutate, "repository alias repository")

    def test_rejects_websocket_repository_access(self) -> None:
        def mutate(source: Path) -> None:
            path = source / "web/ws_handler.rs"
            text = path.read_text()
            marker = "ClientMessage::CreateWebhook {"
            start = text.index(marker)
            arm = text.index("=> {", start) + len("=> {")
            text = text[:arm] + "\n            let _repository = &state.db;" + text[arm:]
            path.write_text(text)

        self.assert_rejected(mutate, "WebSocket arm CreateWebhook")

    def test_rejects_aliased_irc_query_module(self) -> None:
        def mutate(source: Path) -> None:
            path = source / "irc/commands.rs"
            text = "use crate::db::queries::users as user_repository;\n" + path.read_text()
            marker = "pub async fn handle_command("
            start = text.index(marker)
            opening = text.index("{", start)
            text = (
                text[: opening + 1]
                + "\n    let _repository_call = user_repository::get_user;"
                + text[opening + 1 :]
            )
            path.write_text(text)

        self.assert_rejected(mutate, "IRC adapter concord/server/src/irc/commands.rs:handle_command")

    def test_rejects_production_helper_after_cfg_test_module(self) -> None:
        def mutate(source: Path) -> None:
            path = source / "web/rest_api.rs"
            with path.open("a") as output:
                output.write(
                    "\n#[cfg(test)]\nmod late_test_fixture {}\n"
                    "pub async fn late_production_helper() {\n"
                    "    let _ = sqlx::query(\"SELECT 1\");\n"
                    "}\n"
                )

        self.assert_rejected(mutate, "late_production_helper")


if __name__ == "__main__":
    unittest.main()
