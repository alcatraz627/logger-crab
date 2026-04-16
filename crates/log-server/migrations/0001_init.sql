-- logger-crab hot tier — SQLite, single events table + FTS5 mirror.
-- Schema source: PLAN.md §4.2. Changes here must be reflected there first.

CREATE TABLE IF NOT EXISTS events (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    request_id       TEXT NOT NULL,
    event            TEXT NOT NULL,
    severity_number  INTEGER NOT NULL,
    severity_text    TEXT NOT NULL,
    ts               TEXT NOT NULL,
    message          TEXT,

    service          TEXT,
    env              TEXT,

    -- V1.5 identity keys — indexed, optional. See identity-hierarchy.md.
    user_id          TEXT,
    session_id       TEXT,
    client_id        TEXT,

    -- Typed envelope slots stored as JSON; typed indexes extracted below.
    actor_json       TEXT,
    object_json      TEXT,
    state_json       TEXT,
    system_json      TEXT,
    deploy_json      TEXT,
    source_json      TEXT,
    trace_json       TEXT,
    template_json    TEXT,

    payload          TEXT NOT NULL DEFAULT '{}',

    sample_rate      REAL NOT NULL DEFAULT 1.0,
    dropped_count    INTEGER NOT NULL DEFAULT 0,

    ingested_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX IF NOT EXISTS events_ts_idx           ON events (ts);
CREATE INDEX IF NOT EXISTS events_request_id_idx   ON events (request_id);
CREATE INDEX IF NOT EXISTS events_user_id_idx      ON events (user_id);
CREATE INDEX IF NOT EXISTS events_session_id_idx   ON events (session_id);
CREATE INDEX IF NOT EXISTS events_client_id_idx    ON events (client_id);
CREATE INDEX IF NOT EXISTS events_service_env_idx  ON events (service, env);
CREATE INDEX IF NOT EXISTS events_event_idx        ON events (event);
CREATE INDEX IF NOT EXISTS events_severity_idx     ON events (severity_number);

-- FTS5 virtual table mirrors searchable columns for free-text queries.
CREATE VIRTUAL TABLE IF NOT EXISTS events_fts USING fts5(
    event,
    message,
    payload,
    content='events',
    content_rowid='id'
);

CREATE TRIGGER IF NOT EXISTS events_ai AFTER INSERT ON events BEGIN
    INSERT INTO events_fts(rowid, event, message, payload)
    VALUES (new.id, new.event, new.message, new.payload);
END;

CREATE TRIGGER IF NOT EXISTS events_ad AFTER DELETE ON events BEGIN
    INSERT INTO events_fts(events_fts, rowid, event, message, payload)
    VALUES ('delete', old.id, old.event, old.message, old.payload);
END;

CREATE TRIGGER IF NOT EXISTS events_au AFTER UPDATE ON events BEGIN
    INSERT INTO events_fts(events_fts, rowid, event, message, payload)
    VALUES ('delete', old.id, old.event, old.message, old.payload);
    INSERT INTO events_fts(rowid, event, message, payload)
    VALUES (new.id, new.event, new.message, new.payload);
END;

CREATE TABLE IF NOT EXISTS rotation_log (
    id               INTEGER PRIMARY KEY AUTOINCREMENT,
    started_at       TEXT NOT NULL,
    finished_at      TEXT,
    rows_drained     INTEGER,
    s3_key           TEXT,
    error            TEXT
);
