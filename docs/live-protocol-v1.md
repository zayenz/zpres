# Live session protocol v1

This document is the authoritative reference for the local session/control
protocol implemented by `zpres serve`. Protocol v1 is served identically from
the presentation and speaker listener ports. The two pages are views over one
server-owned session; neither page owns an independent navigation or timer
state.

The listeners bind to `127.0.0.1` and do not authenticate clients. This is a
local-machine control protocol, not a LAN or remote-control API. Every session
snapshot includes rendered `notes_html` for its current and next routes, and
the same snapshot is available on both ports. The speaker page is a separate
presenter-oriented surface, but neither its port nor this protocol is a
confidentiality boundary.

## Endpoints

The following methods and paths have the same behavior on both active ports:

| Method | Path | Result |
|---|---|---|
| `GET` | `/zpres/session.json` | Current protocol-v1 session snapshot |
| `POST` | `/zpres/control` | Apply one protocol-v1 command and return the resulting snapshot |
| `GET` | `/events` | Server-sent event stream for session, diagnostics, and render updates |

Using another method returns `405 Method Not Allowed` and an `Allow` header
whose value is the method in the table. The root `GET /` is role-specific HTML,
not part of the versioned JSON protocol. Theme-switcher endpoints are also
live-server facilities, but are outside the session/control v1 contract.

## Control requests

`POST /zpres/control` requires `Content-Type: application/json`; media-type
parameters such as `charset=utf-8` are accepted. The request body limit is
65,536 bytes (64 KiB). Browser-originated requests must have an `Origin` on one
of the active presentation or speaker ports, using either the printed
`127.0.0.1` host or its `localhost` alias. When `Sec-Fetch-Site` is present with
an Origin, its value must be `same-origin`, `same-site`, or `none`.
Requests without either browser security header are accepted for local
non-browser clients; this does not add authentication.

Each body is one JSON object selected by its `command` field:

```json
{"command":"advance"}
{"command":"retreat"}
{"command":"detail-next"}
{"command":"detail-previous"}
{"command":"navigate","generation":3,"section":2,"detail":1,"step":0}
{"command":"timer-start"}
{"command":"timer-pause"}
{"command":"timer-reset"}
```

`navigate` requires the current Deck `generation` and absolute, zero-based
`section` and `detail` coordinates. `step` is zero-based and defaults to `0`
when omitted. These values are non-negative integers; the route must exist in
the named generation. Other commands have no required fields. Protocol v1
accepts and ignores extra object fields.

Every accepted command returns `200 OK` with the authoritative snapshot and
broadcasts the same snapshot as a `session` event. This includes an accepted
no-op, such as `retreat` at the first route, pausing an already paused timer, or
starting an already running timer. An accepted no-op still increments
`revision` exactly once. A rejected command does not increment the revision or
broadcast a session event.

## Session snapshot

The response from `GET /zpres/session.json`, a successful control response,
and the data of a `session` event share this JSON shape:

```json
{
  "protocol_version": 1,
  "generation": 3,
  "revision": 17,
  "current": {
    "section": 2,
    "detail": 0,
    "step": 1,
    "step_count": 2,
    "slide_id": "methods",
    "role": "main",
    "title": "Methods",
    "notes_html": "<p>Explain the control group.</p>"
  },
  "next": {
    "section": 2,
    "detail": 0,
    "step": 2,
    "step_count": 2,
    "slide_id": "methods",
    "role": "main",
    "title": "Methods",
    "notes_html": "<p>Explain the control group.</p>"
  },
  "timer": {
    "status": "running",
    "elapsed_ms": 42000
  }
}
```

Top-level fields are:

- `protocol_version`: the integer `1`;
- `generation`: the monotonically increasing accepted Deck generation;
- `revision`: the monotonically increasing session revision;
- `current`: the authoritative route, or `null` when no valid Deck route is
  installed;
- `next`: the result of `advance`, or `null` at the end of the Deck; and
- `timer`: the authoritative timer snapshot.

A route contains zero-based `section`, `detail`, and `step`; `step_count`, the
highest Step index for that slide; stable `slide_id`; `role` (`main`, `detail`,
or `generated`); optional `title` (the field is omitted when absent); and
sanitized rendered `notes_html` (an empty string when there are no notes).
Generated splash routes use role `generated`. The next route follows advance
semantics, so it is the next Step of the same slide while Steps remain.

The timer `status` is `stopped`, `running`, or `paused`. `elapsed_ms` is a
non-negative integer. Clients may interpolate elapsed time locally while the
timer is running; the server does not publish per-second events.

## HTTP errors

Errors are JSON. Except for conflicts, their shape is:

```json
{"error":"human-readable message"}
```

The live protocol uses these status contracts:

| Status | Applies to | Meaning |
|---|---|---|
| `400 Bad Request` | Shared HTTP parser and control | Malformed HTTP/JSON, an unknown command, or missing/invalid command fields |
| `403 Forbidden` | Control | Browser Origin/fetch-site context fails the active-origin guard |
| `405 Method Not Allowed` | All three endpoints | Wrong endpoint method; response includes `Allow: GET` or `Allow: POST` as applicable |
| `409 Conflict` | Control | `navigate` names a stale generation or a route that does not exist |
| `413 Content Too Large` | Shared HTTP parser | Declared request body exceeds 65,536 bytes, or request metadata exceeds its parser threshold |
| `415 Unsupported Media Type` | Control | A state-changing request is not `application/json` |

A `409` includes the latest authoritative snapshot so the caller can recover:

```json
{"error":"human-readable conflict","session":{"protocol_version":1}}
```

The abbreviated `session` above represents the complete snapshot shape
documented earlier. Other error responses do not include a session snapshot.
No permissive CORS response header is emitted. The parser currently checks a
request line's size after its line read has allocated; enforcing that bound
during the read remains separate deferred work.

## Server-sent events

`GET /events` returns `text/event-stream`, declares `retry: 500`, and emits a
comment heartbeat approximately every 15 seconds while idle. Event data is one
JSON value with these exact fields:

| Event name | Data payload |
|---|---|
| `diagnostics` | `{"html":"<rendered diagnostics>"}` |
| `session` | The complete protocol-v1 session snapshot |
| `reload` | `{"generation":3}` |
| `theme-css` | `{"generation":3,"href":"assets/theme.css?zpres_live=3"}` |
| `assets` | `{"generation":3,"version":3,"themeCssHref":"assets/theme.css?zpres_live=3"}` |
| `render-state` | `{"generation":3}` |

Every new connection receives a bootstrap sequence in this order:
`diagnostics`, `session`, then `render-state`. An accepted control publishes a
`session` event. An accepted valid rebuild publishes current `diagnostics`,
then `session`, then at most one of `reload`, `theme-css`, or `assets`, chosen
for the strongest render action required. An invalid Source rebuild preserves
the Last-good Deck and session and publishes only `diagnostics`.

Delivery is current-state, not an unbounded event log. Each subscriber mailbox
coalesces to the latest diagnostics snapshot, latest session snapshot, and
latest required render generation. When a batch drains, those classes retain
diagnostics → session → render ordering. A slow or disconnected client can
therefore miss intermediate accepted states. The stream has no event IDs and
does not replay `Last-Event-ID`; reconnecting creates a new stream and receives
the authoritative bootstrap sequence. Clients should use the bootstrap
generation/session, or refetch `/zpres/session.json`, and reload when their
rendered generation differs. The elapsed timer is recovered from the session
snapshot and then interpolated locally.

## Compatibility policy

Protocol v1 has no version negotiation, and no protocol v2 endpoint exists
today. Within v1:

- request objects may contain extra fields, which the server ignores;
- clients must ignore unknown additive response fields and unknown SSE event
  names;
- existing fields and events will not be removed or renamed; and
- existing field types and documented semantics will not change.

A removal, rename, type change, or semantic break requires protocol v2 and an
explicit dual-version migration. Adding optional response fields or new event
names is compatible with v1 under the client rules above.
