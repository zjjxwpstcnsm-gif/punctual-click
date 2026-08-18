CREATE TABLE IF NOT EXISTS execution_logs (
    id                      TEXT PRIMARY KEY NOT NULL,
    task_id                 TEXT NOT NULL,
    scheduled_at_ms         INTEGER NOT NULL,
    dispatched_at_ms        INTEGER,
    observed_click_at_ms    INTEGER,
    dispatch_delay_ms       INTEGER,
    observed_delay_ms       INTEGER,
    outcome_json            TEXT NOT NULL,
    final_url               TEXT,
    message                 TEXT NOT NULL,
    error_code              TEXT,
    screenshot_path         TEXT,
    created_at_ms           INTEGER NOT NULL,
    FOREIGN KEY(task_id) REFERENCES click_tasks(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_execution_logs_task_created
    ON execution_logs(task_id, created_at_ms DESC);
