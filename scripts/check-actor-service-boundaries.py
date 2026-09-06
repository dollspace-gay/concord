#!/usr/bin/env python3
"""Guard transport adapters against bypassing actor-owned domain services."""

from __future__ import annotations

import argparse
import re
from pathlib import Path

from architecture.rust_source import function_bodies, mask_rust, module_sources, production_source


# The only storage-layer symbols HTTP adapters may invoke. `upload_file` maps a
# storage-adapter error after actor-owned reservation, while `get_upload` opens
# the already-authorized rooted file. Authentication belongs to the AuthUser and
# BotAuth extractors; no REST handler/helper may reach a domain repository.
REST_STORAGE_ADAPTERS = {
    "upload_file": {"MediaError"},
    "get_upload": {"open_rooted_media"},
}

WS_EXPLICIT_ENGINE_DELEGATION = (
    "CreateWebhook",
    "DeleteWebhook",
    "CreateBot",
    "CreateBotToken",
    "ListBotTokens",
    "DeleteBotToken",
    "CreateOAuth2App",
    "ListOAuth2Apps",
    "DeleteOAuth2App",
    "InvokeSlashCommand",
    "RespondToInteraction",
    "InvokeMessageComponent",
)










def repository_aliases(source: str) -> tuple[set[str], set[str]]:
    """Resolve repository module aliases and directly imported symbols."""
    masked = mask_rust(source)
    modules = {"sqlx"}
    symbols: set[str] = set()
    for match in re.finditer(r"\buse\s+sqlx\s+as\s+(\w+)\s*;", masked):
        modules.add(match.group(1))
    for match in re.finditer(
        r"\buse\s+sqlx\s*::\s*(\w+)\s*(?:as\s+(\w+))?\s*;", masked
    ):
        if match.group(1).startswith("query"):
            symbols.add(match.group(2) or match.group(1))
    for match in re.finditer(r"\buse\s+sqlx\s*::\s*\{(?P<body>.*?)\}\s*;", masked, re.S):
        for item in match.group("body").split(","):
            leaf = re.search(r"(?:^|::)\s*(\w+)\s*(?:as\s+(\w+))?\s*$", item.strip())
            if leaf and leaf.group(1).startswith("query"):
                symbols.add(leaf.group(2) or leaf.group(1))
    for match in re.finditer(
        r"\buse\s+crate\s*::\s*db\s*::\s*queries\s+as\s+(\w+)\s*;", masked
    ):
        modules.add(match.group(1))
    for match in re.finditer(r"\buse\s+crate\s*::\s*db\s+as\s+(\w+)\s*;", masked):
        modules.add(match.group(1))
    if re.search(r"\buse\s+crate\s*::\s*db\s*;", masked):
        modules.add("db")
    if re.search(r"\buse\s+crate\s*::\s*db\s*::\s*queries\s*;", masked):
        modules.add("queries")
    for match in re.finditer(
        r"\buse\s+crate\s*::\s*db\s*::\s*\{\s*queries\s+as\s+(\w+)\s*\}\s*;",
        masked,
    ):
        modules.add(match.group(1))
    for match in re.finditer(
        r"\buse\s+crate\s*::\s*\{\s*db\s+as\s+(\w+)\s*\}\s*;", masked
    ):
        modules.add(match.group(1))
    if re.search(r"\buse\s+crate\s*::\s*\{\s*db\s*\}\s*;", masked):
        modules.add("db")
    if re.search(r"\buse\s+crate\s*::\s*db\s*::\s*\{\s*queries\s*\}\s*;", masked):
        modules.add("queries")
    for match in re.finditer(
        r"\buse\s+crate\s*::\s*db\s*::\s*queries\s*::\s*(\w+)\s*(?:as\s+(\w+))?\s*;",
        masked,
    ):
        modules.add(match.group(2) or match.group(1))
    for match in re.finditer(
        r"\buse\s+crate\s*::\s*db\s*::\s*queries\s*::\s*\w+\s*::\s*(\w+)\s*(?:as\s+(\w+))?\s*;",
        masked,
    ):
        symbols.add(match.group(2) or match.group(1))
    for match in re.finditer(
        r"\buse\s+crate\s*::\s*db\s*::\s*queries\s*::\s*\{(?P<body>.*?)\}\s*;",
        masked,
        re.S,
    ):
        for item in match.group("body").split(","):
            item = item.strip()
            imported = re.fullmatch(r"(?P<path>(?:self|\w+)(?:::\w+)*)\s+as\s+(\w+)", item)
            plain = re.fullmatch(r"((?:\w+::)*\w+)", item)
            if imported:
                target = imported.group(2)
                if "::" in imported.group("path"):
                    symbols.add(target)
                else:
                    modules.add(target)
            elif plain and plain.group(1) != "self":
                target = plain.group(1).rsplit("::", 1)[-1]
                if "::" in plain.group(1):
                    symbols.add(target)
                else:
                    modules.add(target)
    for match in re.finditer(
        r"\buse\s+crate\s*::\s*db\s*::\s*queries\s*::\s*\w+\s*::\s*\{(?P<body>.*?)\}\s*;",
        masked,
        re.S,
    ):
        for item in match.group("body").split(","):
            leaf = re.search(r"(?:^|::)\s*(\w+)\s*(?:as\s+(\w+))?\s*$", item.strip())
            if leaf and leaf.group(1) != "self":
                symbols.add(leaf.group(2) or leaf.group(1))
    return modules, symbols


def repository_hits(
    body: str, module_aliases: set[str], symbol_aliases: set[str]
) -> list[str]:
    masked = mask_rust(body)
    patterns = {
        "crate::db": r"\bcrate\s*::\s*db\s*::",
        "AppState.db": r"\b(?:state|app_state)\s*\.\s*db\b",
        "engine database accessor": r"\.\s*(?:db|get_db)\s*\(",
        "transport write admission": r"\.\s*begin_admitted_write\s*\(",
    }
    for alias in sorted(module_aliases):
        patterns[f"repository alias {alias}"] = rf"\b{re.escape(alias)}\s*::"
    for alias in sorted(symbol_aliases):
        patterns[f"repository symbol {alias}"] = rf"\b{re.escape(alias)}\b"
    return [label for label, pattern in patterns.items() if re.search(pattern, masked)]


def media_symbols(body: str) -> set[str]:
    return set(re.findall(r"\bcrate\s*::\s*media\s*::\s*(\w+)", mask_rust(body)))


def inspect_rest(root: Path, violations: list[str]) -> int:
    functions_seen = 0
    sources = module_sources(root / "concord/server/src/web/rest_api.rs")
    # Children may inherit repository aliases through `use super::...`.
    # Conservatively resolve aliases across the adapter's production modules.
    module_aliases, symbol_aliases = repository_aliases("\n".join(sources.values()))
    for path, source in sources.items():
        functions = function_bodies(source)
        functions_seen += len(functions)
        for function, body in functions.items():
            hits = repository_hits(body, module_aliases, symbol_aliases)
            if hits:
                violations.append(
                    f"HTTP adapter {function} ({path.relative_to(root)}) bypasses actor service ownership via {', '.join(hits)}"
                )
            unexpected_media = sorted(media_symbols(body) - REST_STORAGE_ADAPTERS.get(function, set()))
            if unexpected_media:
                violations.append(
                    f"HTTP adapter {function} uses unapproved storage symbols: {', '.join(unexpected_media)}"
                )
    return functions_seen


def ws_arm_bodies(source: str) -> dict[str, str]:
    masked = mask_rust(source)
    matches = list(re.finditer(r"(?m)^\s*ClientMessage::([A-Z]\w*)\b", masked))
    arms: dict[str, str] = {}
    for index, match in enumerate(matches):
        end = matches[index + 1].start() if index + 1 < len(matches) else len(source)
        arms[match.group(1)] = source[match.start() : end]
    return arms


def inspect_websocket(root: Path, violations: list[str]) -> int:
    entry = root / "concord/server/src/web/ws_handler.rs"
    sources = module_sources(entry)
    dispatch_path = entry.with_suffix("") / "dispatch.rs"
    arms = ws_arm_bodies(sources[dispatch_path])
    # Resolve the typed dispatcher into its domain handler. Validate every
    # command helper as well, so moving a query into a sibling cannot hide it.
    command_sources = {path: source for path, source in sources.items()
                       if path.name not in {"connection.rs", "protocol.rs"}}
    functions = {path: function_bodies(source) for path, source in command_sources.items()}
    aliases = repository_aliases("\n".join(command_sources.values()))
    for path, bodies in functions.items():
        for function, body in bodies.items():
            hits = repository_hits(body, *aliases)
            if hits:
                violations.append(
                    f"WebSocket adapter {path.relative_to(root)}:{function} bypasses actor service ownership via {', '.join(hits)}"
                )
    for variant in WS_EXPLICIT_ENGINE_DELEGATION:
        arm = arms.get(variant)
        if arm is None:
            violations.append(f"missing guarded WebSocket transport arm {variant}")
            continue
        delegates = re.findall(r"\b(\w+)::(\w+)\s*\(", mask_rust(arm))
        bodies = [functions.get(entry.with_suffix("") / f"{module}.rs", {}).get(function, "")
                  for module, function in delegates]
        if not any(re.search(r"\bengine\s*\.", mask_rust(body)) for body in bodies):
            violations.append(f"{variant} does not delegate through ChatEngine")
    return len(arms)


def inspect_irc(root: Path, violations: list[str]) -> None:
    for entry in ("concord/server/src/irc/commands.rs", "concord/server/src/irc/connection.rs"):
        sources = module_sources(root / entry)
        aliases = repository_aliases("\n".join(sources.values()))
        for path, source in sources.items():
            for function, body in function_bodies(source).items():
                hits = repository_hits(body, *aliases)
                if hits:
                    violations.append(
                        f"IRC adapter {path.relative_to(root)}:{function} bypasses actor service ownership via {', '.join(hits)}"
                    )


def inspect_engine_imports(root: Path, violations: list[str]) -> None:
    for path, source in module_sources(root / "concord/server/src/engine/mod.rs").items():
        masked = mask_rust(source)
        for forbidden in ("crate::web::", "crate::irc::"):
            pattern = re.escape(forbidden).replace(r"::", r"\s*::\s*")
            if re.search(pattern, masked):
                violations.append(f"{path.relative_to(root)} imports transport layer {forbidden}")


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--root",
        type=Path,
        default=Path(__file__).resolve().parents[1],
        help="repository root to inspect (used by isolated regression fixtures)",
    )
    args = parser.parse_args()
    root = args.root.resolve()
    violations: list[str] = []
    rest_count = inspect_rest(root, violations)
    ws_count = inspect_websocket(root, violations)
    inspect_irc(root, violations)
    inspect_engine_imports(root, violations)
    if violations:
        raise SystemExit(
            "architecture boundary failed:\n"
            + "\n".join(f"- {item}" for item in violations)
        )
    print(
        "actor-service architecture boundary passed for "
        f"all {rest_count} production HTTP handlers/helpers, all {ws_count} WebSocket operations, "
        "IRC command adapters, and engine transport imports"
    )


if __name__ == "__main__":
    main()
