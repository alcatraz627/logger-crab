# logger-crab — Schema Evolution Strategy

Canonical copy. Project copy at `~/.claude/assets/diagrams/logger-crab-schema-evolution.md`.

Nothing is planned right now, but the architecture has to absorb changes
without migrations-from-hell or wire-format breakage. This doc captures
the rules.

## The three change surfaces

```txt
┌────────────────────────────────────────────────────────────────────┐
│  1. Wire format (LogEvent JSON)        — changed by shipper libs   │
│  2. Storage schema (SQLite + FTS5)     — changed by sqlx migration │
│  3. Query/response API (GET /logs)     — changed by route handler  │
└────────────────────────────────────────────────────────────────────┘
```

Each surface evolves independently. A new field can land in the wire
format long before a storage index exists for it (it lives in `payload`
in the meantime). A storage column can be added without changing the
public response shape. This separation is the main thing that keeps
upgrades low-risk.

## Rule 1 — Wire format is additive only

- New **top-level fields** are allowed but must be optional. The server
  tolerates unknown keys silently.
- New **typed envelope slots** (actor/object/state/system/deploy/source/
  trace) add new sub-keys, never rename or repurpose existing ones.
- **Free-form growth** lives in `payload`. Anything experimental or
  short-lived goes here first; promote to a typed slot only once it
  stabilizes and earns an index.
- **Never reuse a field name** for a different meaning. Once `request_id`
  means one-distributed-call, it is forever one-distributed-call.
- **Never shrink a field's acceptable values.** Adding a new `severity_text`
  value is fine; removing one is not. Shippers predating the change will
  still emit the old value.

## Rule 2 — Storage changes go through sqlx migrations

Every schema change is a new SQL file in `crates/log-server/migrations/`
named `NNNN_<slug>.sql` (sqlx sorts by leading integer). The migrator
runs on every boot:

```rust
sqlx::migrate!("./migrations").run(&pool).await?;
```

sqlx tracks applied migrations in `_sqlx_migrations`, so reboots are
idempotent and partial rollouts are safe.

### Allowed in a migration

- `CREATE INDEX IF NOT EXISTS` — always safe.
- `ALTER TABLE ... ADD COLUMN` — safe in SQLite; column defaults `NULL`
  for existing rows.
- `CREATE TRIGGER` / `CREATE VIRTUAL TABLE` — safe.
- Backfill `UPDATE` — allowed but keep it bounded (index-driven
  predicates, `LIMIT` if the table is large). Long `UPDATE` blocks
  ingest on the same connection.

### Avoid in a migration

- `DROP COLUMN` — SQLite supports this since 3.35 but it rewrites the
  table. Use only with explicit scheduling; prefer leaving a dead column
  until the next rebuild opportunity.
- `DROP INDEX` — cheap, but make sure no in-flight query depends on it.
- Renaming — break into (add new column, backfill, update readers,
  drop old column) across multiple migrations.

### Forward-only

Down-migrations are not tracked. To roll back, write a new migration
that reverts. This matches the mental model of an append-only log store:
history is forward-pointing.

## Rule 3 — FTS5 rebuild rules

The FTS5 virtual table mirrors `event`, `message`, `payload`. If that
mirror changes:

- Add a column → write a migration that `DROP TABLE events_fts` + re-
  `CREATE VIRTUAL TABLE` + `INSERT INTO events_fts (rowid, …) SELECT id,
… FROM events`. Run during a low-traffic window; FTS rebuild on a
  48-hour hot tier is bounded work.
- Triggers `events_ai` / `events_ad` / `events_au` must be updated in
  the same migration so new rows stay coherent.

## Rule 4 — API response shapes are additive too

`GET /logs` and the dashboard responses follow the same additivity rule
as the wire format. New keys may appear; existing keys keep their
meaning. The shipper libs treat unknown server-side keys as tolerable.

## Worked example — identity hierarchy (V1.5)

See `identity-hierarchy.md`. The upgrade plan is:

1. **Wire format:** add `user_id`, `session_id`, `client_id` as optional
   top-level strings. `actor.session` / `actor.client` sub-shapes for
   richer context.
2. **Storage:** already indexed from migration `0001_init.sql`. No new
   migration needed for V1.5 — the columns are pre-provisioned. If
   instead the columns were added later: migration `0002_identity.sql`
   would `ALTER TABLE events ADD COLUMN user_id TEXT` + `CREATE INDEX`.
3. **API:** `GET /logs` gains `user_id`, `session_id` filter params.
   Existing filters untouched.
4. **Shippers:** TS shipper reads `useSession()` + cookie; Python
   shipper reads `request.state.user.id` + `X-Session-ID`.

Because the columns exist from day one, an old binary ingesting new
events silently stores the identity keys and a new binary querying old
events returns `NULL` for them. Both directions work.

## Anti-patterns to reject in code review

- A migration that `DROP`s anything used by the current release.
- A wire-format change that removes a field or narrows a field's type.
- A shipper that stops emitting a field the dashboard currently renders.
- Using `request_id` to span multiple requests — breaks its meaning.
  If the upgrade target is "same user across requests," add a new key
  (see V1.5 session_id plan), don't stretch an existing one.
- Storing UUIDs in `payload.foo_id` after a typed slot exists for them
  — move to `object.*` so indexes and dashboard cards work.

## Decision log

- **2026-04-17** — Pre-provision V1.5 identity columns (`user_id`,
  `session_id`, `client_id`) in `0001_init.sql` so V1.5 ship is a
  no-migration change. Columns are `NULL`-safe and zero cost until
  populated.
- **2026-04-17** — Typed envelope slots stored as JSON (`actor_json`,
  `object_json`, …) rather than flattened columns. Lets us add new
  sub-shapes without migrations; typed indexes over the hot keys
  extracted at insert time.
