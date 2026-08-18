#!/usr/bin/env python3
"""Structural checks that do not require a Rust toolchain."""

from __future__ import annotations

import re
import sqlite3
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def check_workspace() -> None:
    manifest = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))
    assert manifest["workspace"]["package"]["version"] == "0.1.0-alpha.5"
    members = manifest["workspace"]["members"]
    for member in members:
        crate = ROOT / member
        assert (crate / "Cargo.toml").is_file(), member
        assert (crate / "src").is_dir(), member
        assert any((crate / "src" / name).is_file() for name in ("lib.rs", "main.rs")), member


def check_rust_modules() -> None:
    module_pattern = re.compile(
        r"^\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+([A-Za-z_][A-Za-z0-9_]*)\s*;",
        re.M,
    )
    for entry in ROOT.glob("crates/*/src/lib.rs"):
        source = entry.read_text(encoding="utf-8")
        for module in module_pattern.findall(source):
            flat = entry.parent / f"{module}.rs"
            nested = entry.parent / module / "mod.rs"
            assert flat.exists() or nested.exists(), f"missing module {module} declared by {entry}"
    main = ROOT / "crates/punctual-app/src/main.rs"
    source = main.read_text(encoding="utf-8")
    for module in module_pattern.findall(source):
        assert (main.parent / f"{module}.rs").exists(), f"missing app module {module}"


def check_migrations() -> None:
    connection = sqlite3.connect(":memory:")
    connection.execute("PRAGMA foreign_keys = ON")
    for migration in sorted((ROOT / "crates/punctual-storage/migrations").glob("*.sql")):
        connection.executescript(migration.read_text(encoding="utf-8"))
    tables = {
        row[0]
        for row in connection.execute("SELECT name FROM sqlite_master WHERE type='table'")
    }
    assert {"click_tasks", "execution_logs"}.issubset(tables)
    connection.close()


def check_product_contract() -> None:
    dashboard = (ROOT / "crates/punctual-app/src/dashboard.rs").read_text(encoding="utf-8")
    editor = (ROOT / "crates/punctual-app/src/editor.rs").read_text(encoding="utf-8")
    browser = (ROOT / "crates/punctual-browser/src/chromium.rs").read_text(encoding="utf-8")
    discovery = (ROOT / "crates/punctual-browser/src/discovery.rs").read_text(encoding="utf-8")
    worker = (ROOT / "crates/punctual-engine/src/worker.rs").read_text(encoding="utf-8")
    messages = (ROOT / "crates/punctual-core/src/message.rs").read_text(encoding="utf-8")

    for required in (
        "DetectTargets",
        "ValidateManualTarget",
        "HighlightTarget",
        "SaveTask",
        "LoadExecutionLogs",
    ):
        assert required in messages, required

    for required in (
        "resolve_target",
        "dispatch_at_deadline",
        "verify_after_click",
        "save_task_and_log",
        "completion_baseline",
    ):
        assert required in worker, required

    for required in ("inspected_url", "validated_manual_text"):
        assert required in editor, required

    for required in (
        "overflow_y_scrollbar",
        "ClipboardItem::new_string",
        "candidate-list-scroll",
        "task-list-scroll",
        "details-scroll",
        "editor-scroll",
    ):
        assert required in dashboard, required

    assert "completion_baseline" in browser
    assert "ManagedChromium" in discovery
    assert "Chrome wins over the system default" in discovery


def main() -> None:
    check_workspace()
    check_rust_modules()
    check_migrations()
    check_product_contract()
    print("project structure, migrations and alpha.5 product contract: OK")


if __name__ == "__main__":
    main()
