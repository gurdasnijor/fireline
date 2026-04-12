# FQA: Durable-Streams HTTP/SSE Functional

Date: 2026-04-12

Preflight build used an isolated target dir per contention rules:

```sh
CARGO_TARGET_DIR=/tmp/fireline-w13 cargo build --bin fireline-streams
```

Server boot command used for the smoke run:

```sh
PORT=19437 /tmp/fireline-w13/debug/fireline-streams
```

The binary reported:

```text
durable-streams ready at http://127.0.0.1:19437/v1/stream
```

## 1. POST append to a stream key, then verify via subscribe

Verdict: Pass

Driver script:

```sh
curl -X PUT http://127.0.0.1:19437/v1/stream/qa-s1 \
  -H 'Content-Type: text/plain'

curl -N http://127.0.0.1:19437/v1/stream/qa-s1?offset=-1\&live=sse > /tmp/qa-s1.sse &

curl -i -X POST http://127.0.0.1:19437/v1/stream/qa-s1 \
  -H 'Content-Type: text/plain' \
  --data 'alpha'

curl -X POST http://127.0.0.1:19437/v1/stream/qa-s1 \
  -H 'Content-Type: text/plain' \
  -H 'Stream-Closed: true'
```

Observed output:

```text
HTTP/1.1 204 No Content
stream-next-offset: 0000000000000001_0000000000000005
```

Observed SSE:

```text
event: control
data:{"streamNextOffset":"0000000000000000_0000000000000000","streamCursor":"1","upToDate":true}

event: data
data:alpha

event: control
data:{"streamNextOffset":"0000000000000001_0000000000000005","streamCursor":"134217733","upToDate":true}

event: control
data:{"streamNextOffset":"0000000000000001_0000000000000005","upToDate":true,"streamClosed":true}
```

Evidence:

- Append returned `204 No Content`.
- `Stream-Next-Offset` advanced to `0000000000000001_0000000000000005`.
- The subscriber observed the appended payload `alpha` and then a terminal `streamClosed` control event.

## 2. SSE replay from a saved offset

Verdict: Pass

Driver script:

```sh
curl -X PUT http://127.0.0.1:19438/v1/stream/qa-s2 \
  -H 'Content-Type: text/plain'

curl -i -X POST http://127.0.0.1:19438/v1/stream/qa-s2 \
  -H 'Content-Type: text/plain' \
  --data 'one'
# saved Stream-Next-Offset:
# 0000000000000001_0000000000000003

curl -X POST http://127.0.0.1:19438/v1/stream/qa-s2 \
  -H 'Content-Type: text/plain' \
  --data 'two'

curl -X POST http://127.0.0.1:19438/v1/stream/qa-s2 \
  -H 'Content-Type: text/plain' \
  --data 'three'

curl -X POST http://127.0.0.1:19438/v1/stream/qa-s2 \
  -H 'Content-Type: text/plain' \
  -H 'Stream-Closed: true'

curl -i -N \
  'http://127.0.0.1:19438/v1/stream/qa-s2?offset=0000000000000001_0000000000000003&live=sse'
```

Observed output:

```text
HTTP/1.1 200 OK
content-type: text/event-stream

event: data
data:two

event: data
data:three

event: control
data:{"streamNextOffset":"0000000000000003_000000000000000b","upToDate":true,"streamClosed":true}
```

Evidence:

- The replay subscription did not resend `one`.
- It started at the saved next offset and returned only `two` and `three`.
- The terminal control event advanced to the expected next offset for all three appends.

## 3. Two concurrent subscribers

Verdict: Pass

Driver script:

```sh
curl -X PUT http://127.0.0.1:19437/v1/stream/qa-s3 \
  -H 'Content-Type: text/plain'

curl -N 'http://127.0.0.1:19437/v1/stream/qa-s3?offset=-1&live=sse' > /tmp/qa-s3-sub1.sse &
curl -N 'http://127.0.0.1:19437/v1/stream/qa-s3?offset=-1&live=sse' > /tmp/qa-s3-sub2.sse &

curl -X POST http://127.0.0.1:19437/v1/stream/qa-s3 \
  -H 'Content-Type: text/plain' \
  --data 'red'

curl -X POST http://127.0.0.1:19437/v1/stream/qa-s3 \
  -H 'Content-Type: text/plain' \
  --data 'blue'

curl -X POST http://127.0.0.1:19437/v1/stream/qa-s3 \
  -H 'Content-Type: text/plain' \
  -H 'Stream-Closed: true'
```

Observed output:

Subscriber 1:

```text
event: data
data:red

event: data
data:blue
```

Subscriber 2:

```text
event: data
data:red

event: data
data:blue
```

Evidence:

- Both subscribers received the same application data in the same order: `red`, then `blue`.
- Raw SSE bodies were not byte-identical because each subscriber received a different `streamCursor` value in control frames.
- `streamNextOffset` progression matched across both subscribers:
  `0000000000000001_0000000000000003` then `0000000000000002_0000000000000007`.

## 4. Subscribe to a nonexistent stream

Verdict: Pass

Driver script:

```sh
curl -i -N \
  'http://127.0.0.1:19438/v1/stream/qa-s4-does-not-exist?offset=-1&live=sse'
```

Observed output:

```text
HTTP/1.1 404 Not Found
content-type: text/plain; charset=utf-8
cache-control: no-store

Stream not found: qa-s4-does-not-exist
```

Evidence:

- Behavior is not “empty stream”; it is a synchronous `404` before SSE streaming begins.
- Response body is a plain-text error string, not an SSE error frame.

## 5. Append malformed envelope

Verdict: Fail

Interpretation used for this smoke test: at the durable-streams layer, “malformed envelope” means malformed JSON appended to an `application/json` stream. The stream server does not know Fireline envelope semantics; it only validates the stream protocol and JSON-mode body shape.

Driver script:

```sh
curl -X PUT http://127.0.0.1:19438/v1/stream/qa-s5 \
  -H 'Content-Type: application/json'

curl -i -X POST http://127.0.0.1:19438/v1/stream/qa-s5 \
  -H 'Content-Type: application/json' \
  --data '{invalid json}'
```

Observed output:

```text
HTTP/1.1 400 Bad Request
content-type: text/plain; charset=utf-8
cache-control: no-store

Invalid JSON: key must be a string at line 1 column 2
```

Evidence:

- Status code is correct and the message is clear.
- The failure is not returned as a structured JSON error object; it is plain text.
- If the desired contract is “JSON error structure,” the current behavior does not meet it.

## Summary

- Passed: 1, 2, 3, 4
- Failed: 5

Main findings:

- Append and SSE replay behavior are correct for the exercised text-stream cases.
- Offset replay uses the `offset` query parameter and honors the saved `Stream-Next-Offset` header exactly.
- Concurrent subscribers preserve application event order, but control frames include subscriber-local `streamCursor` values, so raw SSE bodies differ.
- Nonexistent-stream SSE is a synchronous `404 Not Found`.
- Invalid JSON append errors are clear, but they are plain text rather than structured JSON.
