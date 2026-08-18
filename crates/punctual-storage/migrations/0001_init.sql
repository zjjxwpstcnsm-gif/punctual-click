PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS click_tasks (
    id                  TEXT PRIMARY KEY NOT NULL,
    title               TEXT NOT NULL,
    url                 TEXT NOT NULL,
    scheduled_at_ms     INTEGER NOT NULL,
    timezone            TEXT NOT NULL,
    click_mode_json     TEXT NOT NULL,
    target_json         TEXT NOT NULL,
    completion_json     TEXT NOT NULL,
    status_json         TEXT NOT NULL,
    result_json         TEXT,
    created_at_ms       INTEGER NOT NULL,
    updated_at_ms       INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_click_tasks_status_schedule
    ON click_tasks(status_json, scheduled_at_ms);
