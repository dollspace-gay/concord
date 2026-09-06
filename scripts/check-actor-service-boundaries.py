#!/usr/bin/env python3
"""Guard transport adapters against bypassing actor-owned domain services."""

from __future__ import annotations

import argparse
import re
from pathlib import Path


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


def mask_rust(source: str) -> str:
    """Mask comments and literals while preserving positions and newlines."""
    chars = list(source)
    index = 0
    block_depth = 0
    state = "code"
    raw_hashes = 0
    while index < len(chars):
        current = source[index]
        following = source[index + 1] if index + 1 < len(source) else ""
        if state == "line_comment":
            if current == "\n":
                state = "code"
            else:
                chars[index] = " "
            index += 1
            continue
        if state == "block_comment":
            if current == "/" and following == "*":
                chars[index] = chars[index + 1] = " "
                block_depth += 1
                index += 2
            elif current == "*" and following == "/":
                chars[index] = chars[index + 1] = " "
                block_depth -= 1
                index += 2
                if block_depth == 0:
                    state = "code"
            else:
                if current != "\n":
                    chars[index] = " "
                index += 1
            continue
        if state == "string":
            if current == "\\":
                chars[index] = " "
                if index + 1 < len(chars):
                    if chars[index + 1] != "\n":
                        chars[index + 1] = " "
                    index += 2
                else:
                    index += 1
            else:
                if current != "\n":
                    chars[index] = " "
                index += 1
                if current == '"':
                    state = "code"
            continue
        if state == "char":
            if current == "\\":
                chars[index] = " "
                if index + 1 < len(chars):
                    chars[index + 1] = " "
                    index += 2
                else:
                    index += 1
            else:
                chars[index] = " "
                index += 1
                if current == "'":
                    state = "code"
            continue
        if state == "raw_string":
            terminator = '"' + ("#" * raw_hashes)
            if source.startswith(terminator, index):
                for offset in range(len(terminator)):
                    chars[index + offset] = " "
                index += len(terminator)
                state = "code"
            else:
                if current != "\n":
                    chars[index] = " "
                index += 1
            continue

        if current == "/" and following == "/":
            chars[index] = chars[index + 1] = " "
            state = "line_comment"
            index += 2
        elif current == "/" and following == "*":
            chars[index] = chars[index + 1] = " "
            block_depth = 1
            state = "block_comment"
            index += 2
        elif current == '"':
            chars[index] = " "
            state = "string"
            index += 1
        elif current == "'" and following and not (following.isalpha() or following == "_"):
            chars[index] = " "
            state = "char"
            index += 1
        elif current in {"r", "b"}:
            raw = re.match(r'(?:br|rb|r)(?P<hashes>#{0,255})"', source[index:])
            if raw:
                token = raw.group(0)
                raw_hashes = len(raw.group("hashes"))
                for offset in range(len(token)):
                    chars[index + offset] = " "
                index += len(token)
                state = "raw_string"
            else:
                index += 1
        else:
            index += 1
    return "".join(chars)


def matching_brace(masked: str, opening: int) -> int | None:
    depth = 0
    for index in range(opening, len(masked)):
        if masked[index] == "{":
            depth += 1
        elif masked[index] == "}":
            depth -= 1
            if depth == 0:
                return index
    return None


def production_source(source: str) -> str:
    """Remove each cfg(test) item without truncating later production items."""
    masked = mask_rust(source)
    output = list(source)
    attribute = re.compile(r"#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]")
    for match in attribute.finditer(masked):
        opening = masked.find("{", match.end())
        semicolon = masked.find(";", match.end())
        if opening < 0 or (0 <= semicolon < opening):
            end = semicolon + 1 if semicolon >= 0 else match.end()
        else:
            closing = matching_brace(masked, opening)
            end = len(masked) if closing is None else closing + 1
        for index in range(match.start(), end):
            if output[index] != "\n":
                output[index] = " "
    return "".join(output)


def function_bodies(source: str) -> dict[str, str]:
    masked = mask_rust(source)
    functions: dict[str, str] = {}
    signature = re.compile(
        r"(?m)^\s*(?:pub(?:\([^)]*\))?\s+)?(?:const\s+)?(?:async\s+)?(?:unsafe\s+)?fn\s+(\w+)\b"
    )
    for match in signature.finditer(masked):
        opening = masked.find("{", match.end())
        if opening < 0:
            continue
        closing = matching_brace(masked, opening)
        if closing is not None:
            functions[match.group(1)] = source[match.start() : closing + 1]
    return functions


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
    relative = "concord/server/src/web/rest_api.rs"
    source = production_source((root / relative).read_text(encoding="utf-8"))
    module_aliases, symbol_aliases = repository_aliases(source)
    functions = function_bodies(source)
    for function, body in functions.items():
        hits = repository_hits(body, module_aliases, symbol_aliases)
        if hits:
            violations.append(
                f"HTTP adapter {function} bypasses actor service ownership via {', '.join(hits)}"
            )
        unexpected_media = sorted(
            media_symbols(body) - REST_STORAGE_ADAPTERS.get(function, set())
        )
        if unexpected_media:
            violations.append(
                f"HTTP adapter {function} uses unapproved storage symbols: {', '.join(unexpected_media)}"
            )
    return len(functions)


def ws_arm_bodies(source: str) -> dict[str, str]:
    masked = mask_rust(source)
    matches = list(re.finditer(r"(?m)^\s*ClientMessage::([A-Z]\w*)\b", masked))
    arms: dict[str, str] = {}
    for index, match in enumerate(matches):
        end = matches[index + 1].start() if index + 1 < len(matches) else len(source)
        arms[match.group(1)] = source[match.start() : end]
    return arms


def inspect_websocket(root: Path, violations: list[str]) -> int:
    relative = "concord/server/src/web/ws_handler.rs"
    source = production_source((root / relative).read_text(encoding="utf-8"))
    module_aliases, symbol_aliases = repository_aliases(source)
    arms = ws_arm_bodies(source)
    for variant in WS_EXPLICIT_ENGINE_DELEGATION:
        body = arms.get(variant)
        if body is None:
            violations.append(f"missing guarded WebSocket transport arm {variant}")
        elif not re.search(r"\bengine\s*\.", mask_rust(body)):
            violations.append(f"{variant} does not delegate through ChatEngine")
    for variant, body in arms.items():
        hits = repository_hits(body, module_aliases, symbol_aliases)
        if hits:
            violations.append(
                f"WebSocket arm {variant} bypasses actor service ownership via {', '.join(hits)}"
            )
    return len(arms)


def inspect_irc(root: Path, violations: list[str]) -> None:
    for relative in (
        "concord/server/src/irc/commands.rs",
        "concord/server/src/irc/connection.rs",
    ):
        source = production_source((root / relative).read_text(encoding="utf-8"))
        module_aliases, symbol_aliases = repository_aliases(source)
        for function, body in function_bodies(source).items():
            hits = repository_hits(body, module_aliases, symbol_aliases)
            if hits:
                violations.append(
                    f"IRC adapter {relative}:{function} bypasses actor service ownership via {', '.join(hits)}"
                )


def inspect_engine_imports(root: Path, violations: list[str]) -> None:
    for engine_file in (root / "concord/server/src/engine").glob("*.rs"):
        source = production_source(engine_file.read_text(encoding="utf-8"))
        masked = mask_rust(source)
        for forbidden in ("crate::web::", "crate::irc::"):
            pattern = re.escape(forbidden).replace(r"::", r"\s*::\s*")
            if re.search(pattern, masked):
                violations.append(f"{engine_file.name} imports transport layer {forbidden}")


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
