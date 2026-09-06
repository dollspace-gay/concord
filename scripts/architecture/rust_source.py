"""Position-preserving Rust source inspection for architecture guards."""
from __future__ import annotations

import re
from pathlib import Path

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
        elif current == "'":
            # A character literal can start with a letter or underscore. Only a
            # complete literal is masked; Rust lifetimes remain code.
            literal = re.match(r"'(?:[^'\\\n]|\\(?:[^\n]|u\{[0-9a-fA-F_]+\}))'", source[index:])
            if literal:
                end = index + len(literal.group(0))
                chars[index:end] = " " * (end - index)
                index = end
            else:
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


def module_sources(entry: Path) -> dict[Path, str]:
    """Follow declared production modules, excluding cfg(test) subtrees."""
    pending = [entry]
    found: dict[Path, str] = {}
    while pending:
        path = pending.pop()
        if path in found:
            continue
        source = production_source(path.read_text(encoding="utf-8"))
        found[path] = source
        base = path.parent if path.name == "mod.rs" else path.with_suffix("")
        for name in re.findall(r"\bmod\s+(\w+)\s*;", mask_rust(source)):
            candidates = (base / f"{name}.rs", base / name / "mod.rs")
            child = next((candidate for candidate in candidates if candidate.is_file()), None)
            if child is None:
                raise ValueError(f"declared module {name} is missing below {path}")
            pending.append(child)
    return found
