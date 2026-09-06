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
            path = source / "web/rest_api/accounts.rs"
            text = path.read_text()
            marker = "pub async fn get_me("
            start = text.index(marker)
            opening = text.index("{", start)
            text = text[: opening + 1] + "\n    let _repository = &state.db;" + text[opening + 1 :]
            path.write_text(text)

        self.assert_rejected(mutate, "HTTP adapter get_me")

    def test_rejects_aliased_http_query_module(self) -> None:
        def mutate(source: Path) -> None:
            path = source / "web/rest_api/accounts.rs"
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
            path = source / "web/rest_api/accounts.rs"
            text = "use sqlx::query as q;\n" + path.read_text()
            marker = "pub async fn get_me("
            start = text.index(marker)
            opening = text.index("{", start)
            text = text[: opening + 1] + '\n    let _query = q("SELECT 1");' + text[opening + 1 :]
            path.write_text(text)

        self.assert_rejected(mutate, "repository symbol q")

    def test_rejects_directly_imported_query_function_item(self) -> None:
        def mutate(source: Path) -> None:
            path = source / "web/rest_api/accounts.rs"
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
            path = source / "web/rest_api/accounts.rs"
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
            path = source / "web/rest_api/accounts.rs"
            text = "use crate::db::{queries as repo};\n" + path.read_text()
            marker = "pub async fn get_user_profile("
            start = text.index(marker)
            opening = text.index("{", start)
            text = text[: opening + 1] + "\n    let _repository = repo::users::get_user;" + text[opening + 1 :]
            path.write_text(text)

        self.assert_rejected(mutate, "repository alias repo")

    def test_rejects_plain_database_queries_import_in_late_helper(self) -> None:
        def mutate(source: Path) -> None:
            path = source / "web/rest_api/accounts.rs"
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
            path = source / "web/rest_api/accounts.rs"
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
            path = source / "web/ws_handler/webhooks.rs"
            text = path.read_text()
            marker = "pub(super) async fn create_webhook("
            start = text.index(marker)
            arm = text.index("{", start) + 1
            text = text[:arm] + "\n            let _repository = &state.db;" + text[arm:]
            path.write_text(text)

        self.assert_rejected(mutate, "WebSocket adapter concord/server/src/web/ws_handler/webhooks.rs:create_webhook")

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

    def test_rejects_child_inheriting_parent_repository_alias(self) -> None:
        def mutate(source: Path) -> None:
            parent = source / "web/rest_api.rs"
            parent.write_text("use crate::db::queries as repository;\n" + parent.read_text())
            child = source / "web/rest_api/accounts.rs"
            child.write_text(child.read_text() + (
                "\nuse super::repository;\n"
                "fn inherited_probe() { let _ = repository::users::get_user; }\n"
            ))

        self.assert_rejected(mutate, "repository alias repository")

    def test_rejects_nested_irc_helper_after_character_literal(self) -> None:
        def mutate(source: Path) -> None:
            parent = source / "irc/commands.rs"
            parent.write_text(parent.read_text() + "\nmod boundary_probe;\n")
            child = source / "irc/commands/boundary_probe.rs"
            child.parent.mkdir(exist_ok=True)
            child.write_text("fn nested_probe() { let marker = '_'; sqlx::query(\"SELECT 1\"); }\n")

        self.assert_rejected(mutate, "nested_probe")

    def test_rejects_nested_engine_transport_dependency(self) -> None:
        def mutate(source: Path) -> None:
            parent = source / "engine/mod.rs"
            parent.write_text(parent.read_text() + "\nmod boundary_probe;\n")
            child = source / "engine/boundary_probe.rs"
            child.write_text("use crate::web::AppState;\n")

        self.assert_rejected(mutate, "imports transport layer crate::web::")

    def test_ignores_test_only_nested_transport_dependency(self) -> None:
        def mutate(source: Path) -> None:
            parent = source / "engine/mod.rs"
            parent.write_text(parent.read_text() + "\n#[cfg(test)]\nmod boundary_probe;\n")
            child = source / "engine/boundary_probe.rs"
            child.write_text("use crate::web::AppState;\n")

        result = self.run_checker(mutate)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_rejects_production_helper_after_cfg_test_module(self) -> None:
        def mutate(source: Path) -> None:
            path = source / "web/rest_api/accounts.rs"
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
