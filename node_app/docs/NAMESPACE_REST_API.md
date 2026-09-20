# Namespace Registry REST API

This document describes the REST endpoints for reading **custom namespace** registrations from the Eld node app.

Custom namespaces let accounts register a slug (e.g. `peter`) that maps to scope `@peter` for future content paths. These endpoints expose the on-chain registry at `/@eld/namespace/{namespace_slug}`.

These endpoints are intended for **explorer/UI** integrations and do not require internal ABCI query calls.

## Base URL

- Node app HTTP API base: `http://<host>:<app_port>`
- Default local app port is often **9001** (see node `config.json` / `consensus_config.json`).

All paths below are relative to that base (no trailing slash required).

## Concepts

| Term | Example | Meaning |
|------|---------|---------|
| `namespace_slug` | `peter` | Canonical registry key (lowercase, validated) |
| `scope` | `@peter` | User-facing scope name (`@` + slug) |
| `registry_path` | `/@eld/namespace/peter` | On-chain immutable CADO path for the registration record |
| `owner` | `0xabc…` | Registrant address at registration time (hex with `0x` prefix) |
| `registered_height` | `1200` | Block height when the namespace was registered |

### Slug rules (path and query input)

Input slugs are **normalized** server-side: trim whitespace, lowercase ASCII.

- Length: **3–32** characters
- Characters: lowercase letters `a–z`, digits `0–9`, single hyphens `-`
- Must not start or end with `-`
- Must not contain `--`

Reserved slugs (e.g. `eld`, `pinboard`, `namespace`) cannot be registered on-chain but may still appear in validation errors elsewhere.

---

## Endpoints

### 1) List registered custom namespaces

`GET /v1/namespaces`

Returns a **paginated** list of all custom namespaces currently registered on the node, including registrations staged in the **current block** (not yet committed to RocksDB).

#### Query parameters

| Parameter | Required | Default | Max | Description |
|-----------|----------|---------|-----|-------------|
| `limit` | no | `50` | `100` | Page size |
| `after_registered_height` | no* | — | — | Continuation: height of last item on previous page |
| `after_namespace_slug` | no* | — | — | Continuation: slug of last item on previous page |

\* **Both** `after_registered_height` and `after_namespace_slug` must be sent together when continuing a list, or **neither** for the first page.

#### Sort order

Newest registrations first:

1. `registered_height` **descending**
2. `namespace_slug` **ascending** (tiebreak when multiple namespaces share the same height)

#### Continuation semantics

After the first page, pass the **last row** from the previous response as the cursor. The server returns rows **strictly older** than that cursor in the sort order above:

- lower `registered_height`, or
- same `registered_height` and **lexicographically greater** `namespace_slug`

This matches the cursor style used by `GET /transactions` (`after_height` + `after_index`).

#### Response shape (`200 OK`)

```json
{
  "namespaces": [
    {
      "namespace_slug": "peter",
      "scope": "@peter",
      "owner": "0xe17404c417fa10cc04fdf73604fcacca8d0a687c",
      "registered_height": 1200,
      "registry_path": "/@eld/namespace/peter"
    }
  ],
  "pagination": {
    "limit": 50,
    "has_next": true,
    "total": 42
  }
}
```

| Field | Type | Description |
|-------|------|-------------|
| `namespaces` | array | Rows for this page (same fields as detail view, without `registered`) |
| `pagination.limit` | number | Effective page size for this response |
| `pagination.has_next` | boolean | `true` if more rows exist after this page |
| `pagination.total` | number \| omitted | Total registered count on the **first** page only; omitted on continuation requests |

#### Explorer examples

- First page (newest first):

  `GET /v1/namespaces?limit=50`

- Smaller page:

  `GET /v1/namespaces?limit=20`

- Next page (use values from the **last** item of the previous page):

  `GET /v1/namespaces?limit=50&after_registered_height=100&after_namespace_slug=alpha`

---

### 2) Get one custom namespace (detail)

`GET /v1/namespace/{namespace_slug}`

Looks up a single namespace by slug. The path segment is normalized the same way as list continuation slugs (trim + lowercase).

Includes registrations staged in the **current block** before they are persisted to RocksDB.

#### Path parameters

| Parameter | Description |
|-----------|-------------|
| `namespace_slug` | User-entered slug, e.g. `Peter`, `peter`, or `peter` (normalized to canonical form) |

#### Response when registered (`200 OK`)

```json
{
  "registered": true,
  "namespace_slug": "peter",
  "scope": "@peter",
  "owner": "0xe17404c417fa10cc04fdf73604fcacca8d0a687c",
  "registered_height": 1200,
  "registry_path": "/@eld/namespace/peter"
}
```

#### Response when not registered (`404 Not Found`)

The response body is JSON (not an empty 404):

```json
{
  "registered": false,
  "namespace_slug": "peter"
}
```

Use `registered === false` to show a “not found” or “available to register” state in the explorer.

#### Explorer examples

- Detail page URL slug `peter`:

  `GET /v1/namespace/peter`

- Case-insensitive input:

  `GET /v1/namespace/Peter` → same canonical `peter`

---

## Error responses (non-2xx)

Most API errors use a standardized JSON envelope:

```json
{
  "code": "BAD_REQUEST",
  "message": "Invalid namespace_slug",
  "details": "…",
  "request_id": null,
  "timestamp": "2026-06-04T12:00:00Z"
}
```

### List endpoint (`GET /v1/namespaces`)

| Condition | HTTP status |
|-----------|-------------|
| `limit` is `0` | `400 Bad Request` |
| Only one of `after_registered_height` / `after_namespace_slug` set | `400 Bad Request` |
| Invalid `after_namespace_slug` (fails slug rules) | `400 Bad Request` |
| Server/storage failure | `500 Internal Server Error` |

### Detail endpoint (`GET /v1/namespace/{namespace_slug}`)

| Condition | HTTP status | Body |
|-----------|-------------|------|
| Invalid slug format | `400 Bad Request` | `ApiErrorResponse` |
| Namespace not registered | `404 Not Found` | `{ "registered": false, "namespace_slug": "…" }` |
| Server failure | `500 Internal Server Error` | `ApiErrorResponse` |

---

## UI mapping recommendations (Eld chain explorer)

### Namespaces list page

1. **Initial load:** `GET /v1/namespaces?limit=50`
2. **Render table/cards** from `namespaces[]`:
   - Primary label: `scope` (e.g. `@peter`) or `namespace_slug`
   - Link target: `/namespaces/{namespace_slug}` (explorer route)
   - Secondary: `owner` (link to account page), `registered_height` (link to block page)
3. **Pagination:**
   - If `pagination.has_next === true`, load more using the **last** row’s `registered_height` and `namespace_slug` as cursor query params.
   - Optionally show `pagination.total` on the first page as “N namespaces registered”.
4. **Empty state:** `namespaces.length === 0` and `pagination.total === 0` (first page).

**Load-more example (pseudo-code):**

```text
last = response.namespaces[response.namespaces.length - 1]
nextUrl = `/v1/namespaces?limit=50&after_registered_height=${last.registered_height}&after_namespace_slug=${last.namespace_slug}`
```

### Namespace detail page

1. Read `namespace_slug` from the explorer route (e.g. `/namespaces/peter`).
2. **Fetch:** `GET /v1/namespace/{namespace_slug}`
3. **If `200` and `registered === true`:**
   - Title: `scope` (`@peter`)
   - Fields: `owner`, `registered_height`, `registry_path`
   - Optional: link `registry_path` to a generic CADO viewer if the explorer supports it
4. **If `404` and body `registered === false`:**
   - Show “Namespace not registered” with canonical `namespace_slug`
5. **If `400`:** show validation error from `message` / `details`

### List row → detail navigation

List items omit `registered` (always registered). Use `namespace_slug` from the list row for the detail request; do not rely on display casing—slugs in responses are already canonical.

### Related APIs (out of scope here)

- Registering a namespace: on-chain `AddNamespace` transaction (CLI/wallet), not these read endpoints.
- Content under `/@{slug}/…`: not implemented in this API phase; only registry metadata is exposed.

---

## Type reference (shared JSON types)

Defined in `chain/common/src/namespace_api.rs` for clients and the node:

| Type | Used in |
|------|---------|
| `NamespaceListItem` | List rows |
| `NamespaceListPagination` | List `pagination` |
| `NamespaceListResponse` | Full list response |
| `NamespaceRegisteredResponse` | Detail `200` |
| `NamespaceNotRegisteredResponse` | Detail `404` |

---

## Comparison with transaction list pagination

| | `GET /transactions` | `GET /v1/namespaces` |
|--|----------------------|----------------------|
| Page size param | `limit` | `limit` |
| Cursor | `after_height` + `after_index` | `after_registered_height` + `after_namespace_slug` |
| Sort | Newest block first | Newest `registered_height` first |
| `total` on first page | yes (when indexer enabled) | yes |
| `total` on continuation | omitted | omitted |
| Requires indexer | yes | no |
