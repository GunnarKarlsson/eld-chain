# Pinboard REST Content API

This document describes the REST endpoints for reading pinboard post metadata and offchain blob content from the Eld node app.

These endpoints are intended for explorer/UI integrations and do not require internal ABCI query calls.

## Base URL

- Node app HTTP API base: `http://<host>:<app_port>`

## Endpoints

### 1) List posts (global or by wallet)

`GET /v1/pinboard/posts`

#### Query params

- `order` (optional): `desc` (default) or `asc`
- `page` (optional): 0-based page index, default `0`
- `page_size` (optional): default `100`, max `100`
- `wallet` (optional): wallet address (`0x...`) to filter by account
- `include_expired` (optional): `true` (default) or `false`

#### Behavior

- Returns metadata + blob payload (`message_b64`) when available.
- For expired posts: `message_b64` is `null`, `blob_status` is `"expired"`.
- For live posts with missing local blob: `message_b64` is `null`, `blob_status` is `"missing"`.
- For live posts with local blob: `blob_status` is `"available"`.

#### Response shape

```json
{
  "items": [
    {
      "cado_path": "/@eld/pinboard/post/0xabc.../msg-123",
      "meta": {
        "message_id": "msg-123",
        "original_signer": "0xabc...",
        "content_key": "3b2f...",
        "expires_height": 1200,
        "visibility": "public",
        "topic": "general",
        "tags": ["news", "release"],
        "committed_height": 200,
        "received_timestamp": 1715000000
      },
      "message_b64": "aGVsbG8gd29ybGQ=",
      "blob_status": "available"
    }
  ],
  "pagination": {
    "page": 0,
    "page_size": 100,
    "has_more": true,
    "next_cursor": "1:100"
  }
}
```

#### Explorer examples

- Global latest first:
  - `GET /v1/pinboard/posts?order=desc&page=0&page_size=50`
- Global oldest first:
  - `GET /v1/pinboard/posts?order=asc&page=0&page_size=50`
- One account only:
  - `GET /v1/pinboard/posts?wallet=0x1111111111111111111111111111111111111111&page=0&page_size=50`
- Exclude expired:
  - `GET /v1/pinboard/posts?include_expired=false&page=0&page_size=50`

### 2) Get one post by pinboard CADO path

`GET /v1/pinboard/post`

#### Query params

- `path` (required): `/@eld/pinboard/post/{wallet}/{message_id}`

#### Response shape

```json
{
  "cado_path": "/@eld/pinboard/post/0xabc.../msg-123",
  "meta": {
    "message_id": "msg-123",
    "original_signer": "0xabc...",
    "content_key": "3b2f...",
    "expires_height": 1200,
    "visibility": "public",
    "topic": "general",
    "tags": ["news", "release"],
    "committed_height": 200,
    "received_timestamp": 1715000000
  },
  "message_b64": "aGVsbG8gd29ybGQ=",
  "blob_status": "available"
}
```

#### Explorer example

- `GET /v1/pinboard/post?path=/@eld/pinboard/post/0x1111111111111111111111111111111111111111/msg-123`

## Error Notes

- Invalid `page_size` (0 or >100): `400 BAD_REQUEST`
- Invalid wallet format in filter/path: `400 BAD_REQUEST`
- Invalid pinboard path format: `400 BAD_REQUEST`
- Missing metadata for single-post lookup: `404 NOT_FOUND`
- If wallet in `path` does not match post owner: `404 NOT_FOUND`

## UI Mapping Recommendations (Explorer)

- Feed page:
  - Call `/v1/pinboard/posts?order=desc&page=<n>&page_size=<k>`
- Account page:
  - Call `/v1/pinboard/posts?wallet=<address>&order=desc&page=<n>&page_size=<k>`
- Detail page:
  - Use `cado_path` from list response and call `/v1/pinboard/post?path=<cado_path>`
- Render content:
  - If `blob_status=="available"` and `message_b64!=null`, decode base64 for display
  - If `blob_status=="expired"` or `"missing"`, show metadata-only state in UI
