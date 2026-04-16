# logger-crab — Identity Hierarchy

Three keys, three query scopes. `request_id` alone cannot link a user's activity across
multiple requests or sessions — that requires `user_id` and `session_id` as separate
top-level indexed fields on every event.

## Diagram

```txt
┌─────────────────────────────────────────────────────────────────────────┐
│  user_id = "u_aakarsh"         (persistent — set on auth, never resets) │
│  ┌───────────────────────────────────────────────────────────────────┐  │
│  │  session_id = "s_3f7b"     (one browser visit, ~hours)            │  │
│  │  ┌──────────────────────┐  ┌──────────────────────┐               │  │
│  │  │ request_id = r_a12   │  │ request_id = r_b34   │  ← requests   │  │
│  │  │ (open /jobs page)    │  │ (upload file)        │    same visit │  │
│  │  │  • ui.pageview       │  │  • files.upload.start│               │  │
│  │  │  • api.jobs.list     │  │  • api.files.put     │               │  │
│  │  │  • db.jobs.query     │  │  • worker.thumb.gen  │               │  │
│  │  └──────────────────────┘  └──────────────────────┘               │  │
│  └───────────────────────────────────────────────────────────────────┘  │
│                                                                         │
│  ┌───────────────────────────────────────────────────────────────────┐  │
│  │  session_id = "s_9c2e"     (next day — same user, new login)      │  │
│  │  ┌──────────────────────┐                                         │  │
│  │  │ request_id = r_c56   │                                         │  │
│  │  │  • ui.pageview       │                                         │  │
│  │  │  • api.billing.view  │                                         │  │
│  │  └──────────────────────┘                                         │  │
│  └───────────────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────────────┘
```

## Queries

| Scope               | SQL predicate                                       |
| ------------------- | --------------------------------------------------- |
| One request         | `WHERE request_id = 'r_a12'`                        |
| One visit           | `WHERE session_id = 's_3f7b'`                       |
| All of a user       | `WHERE user_id = 'u_aakarsh'`                       |
| User this week      | `WHERE user_id = 'u_aakarsh' AND ts > now() - '7d'` |
| Anon user by device | `WHERE client_id = 'c_d8e1' AND user_id IS NULL`    |

## Schema implication

Add to the V1 event schema (section 4.1 of PLAN.md) as **top-level, indexed, optional**:

```ts
user_id?: string;       // "u_<nanoid>" — null for anonymous visitors
session_id?: string;    // "s_<nanoid>" — per browser visit (TTL ~24h inactivity)
client_id?: string;     // "c_<nanoid>" — persistent device/browser cookie
```

All three are nullable because emitters like `Cron Jobs` run without a user context.
The shipper libs read the values from environment-appropriate sources:

| Emitter        | user_id source                             | session_id source                        |
| -------------- | ------------------------------------------ | ---------------------------------------- |
| Next.js (UI)   | `useSession().user.id`                     | cookie `sid` (shipper sets on mount)     |
| FastAPI        | auth dependency → `request.state.user.id`  | header `X-Session-ID` propagated from UI |
| Credit Worker  | payload field `enqueued_by` (from FastAPI) | payload field `session_id`               |
| Cron / Scripts | `null`                                     | `null`                                   |

## Handoff rules

- **UI → FastAPI:** UI sets `X-Session-ID` and authenticated cookie. FastAPI reads both
  and adds them to its scope.
- **FastAPI → Redis:** When enqueueing a worker event, include `{ user_id, session_id,
request_id }` in the payload. Do NOT generate new ids — inherit.
- **Worker → logger-crab:** Shipper sees the inherited ids in its own scope and tags
  every emit with them. The worker's request_id may change per-job, but user_id and
  session_id stay constant for the triggering flow.

## Why three keys, not one

- `request_id` is **narrow** — its whole value is "show me one exact distributed call."
  Reusing it across requests destroys that property (you couldn't trace a single flow).
- `user_id` is **too broad** for debugging — "show me everything this user ever did"
  returns too much noise for one incident.
- `session_id` is the **middle ground** — "what happened in the visit where the bug
  occurred" is the real unit of support-ticket debugging.

## Canonical copy

Mirrored at `~/.claude/assets/diagrams/logger-crab-identity-hierarchy.md`.
