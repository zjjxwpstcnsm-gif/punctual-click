#!/usr/bin/env python3
"""Lightweight truncation check for Rust source files.

This is intentionally not presented as a compiler or parser. It only verifies
that delimiters remain balanced while ignoring comments and common Rust string
forms. Cargo remains the authoritative validation step.
"""

from __future__ import annotations

from pathlib import Path

PAIRS = {"(": ")", "{": "}", "[": "]"}


def check(path: Path) -> None:
    source = path.read_text(encoding="utf-8")
    stack: list[tuple[str, int]] = []
    index = 0
    state = "code"
    block_depth = 0
    raw_hashes = 0

    while index < len(source):
        char = source[index]
        next_char = source[index + 1] if index + 1 < len(source) else ""

        if state == "code":
            if char == "/" and next_char == "/":
                state = "line_comment"
                index += 2
                continue
            if char == "/" and next_char == "*":
                state = "block_comment"
                block_depth = 1
                index += 2
                continue
            if char == '"' or (char == "b" and next_char == '"'):
                state = "string"
                index += 2 if char == "b" else 1
                continue
            if char == "r" or (char == "b" and next_char == "r"):
                cursor = index + (1 if char == "r" else 2)
                hashes = 0
                while cursor < len(source) and source[cursor] == "#":
                    hashes += 1
                    cursor += 1
                if cursor < len(source) and source[cursor] == '"':
                    state = "raw_string"
                    raw_hashes = hashes
                    index = cursor + 1
                    continue
            if char in PAIRS:
                stack.append((char, index))
            elif char in PAIRS.values():
                if not stack:
                    raise ValueError(f"unexpected {char!r} at byte {index}")
                opening, opening_index = stack.pop()
                if PAIRS[opening] != char:
                    raise ValueError(
                        f"mismatched {opening!r} at byte {opening_index} "
                        f"and {char!r} at byte {index}"
                    )
            index += 1
            continue

        if state == "line_comment":
            if char == "\n":
                state = "code"
            index += 1
            continue

        if state == "block_comment":
            if char == "/" and next_char == "*":
                block_depth += 1
                index += 2
            elif char == "*" and next_char == "/":
                block_depth -= 1
                index += 2
                if block_depth == 0:
                    state = "code"
            else:
                index += 1
            continue

        if state == "string":
            if char == "\\":
                index += 2
            elif char == '"':
                state = "code"
                index += 1
            else:
                index += 1
            continue

        if state == "raw_string":
            terminator = '"' + ("#" * raw_hashes)
            if source.startswith(terminator, index):
                index += len(terminator)
                state = "code"
            else:
                index += 1

    if state not in {"code", "line_comment"}:
        raise ValueError(f"unterminated lexical state: {state}")
    if stack:
        opening, opening_index = stack[-1]
        raise ValueError(f"unclosed {opening!r} at byte {opening_index}")


def main() -> None:
    paths = sorted(Path("crates").rglob("*.rs"))
    for path in paths:
        check(path)
        print(f"RUST DELIMITERS OK: {path}")


if __name__ == "__main__":
    main()
