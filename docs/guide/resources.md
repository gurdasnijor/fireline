# Resources

## Client-side resource helpers

The current helper functions live in [packages/client/src/resources.ts](../../packages/client/src/resources.ts):

- `localPath(path, mountPath, readOnly?)`
- `streamBlob(stream, key, mountPath)`
- `gitRepo(url, ref, mountPath)`
- `ociImage(image, path, mountPath)`
- `httpUrl(url, mountPath)`

Example:

```ts
import { agent, compose, middleware, sandbox } from '@fireline/client'
import { trace } from '@fireline/client/middleware'
import { gitRepo, streamBlob } from '@fireline/client/resources'

const handle = await compose(
  sandbox({
    resources: [
      gitRepo('https://github.com/durable-streams/durable-streams', 'main', '/workspace/upstream'),
      streamBlob('resources:tenant-default', 'codebase', '/workspace/blob.tar'),
    ],
  }),
  middleware([trace()]),
  agent(['../../target/debug/fireline-testy']),
).start({
  serverUrl: 'http://127.0.0.1:4440',
  name: 'resource-demo',
})
```

## What the client actually serializes

The client type `ResourceSourceRef` also includes variants for:

- `s3`
- `gcs`
- `dockerVolume`
- `streamFs`

Those types exist in [packages/client/src/resources.ts](../../packages/client/src/resources.ts), but there are no helper constructors for them yet.

## Mount timing

Resources are prepared before the agent starts.

You can see that in provider implementations:

- local subprocess: [crates/fireline-sandbox/src/providers/local_subprocess.rs](../../crates/fireline-sandbox/src/providers/local_subprocess.rs)
- Docker: [crates/fireline-sandbox/src/providers/docker.rs](../../crates/fireline-sandbox/src/providers/docker.rs)

Both call `prepare_resources(...)` before the agent process is launched.

## Resource mounting backends

The mounting layer lives in [crates/fireline-resources/src/mounter.rs](../../crates/fireline-resources/src/mounter.rs).

Notable current behavior:

- local subprocess always wires `LocalPathMounter`
- Docker also uses `prepare_resources(...)`
- durable-stream blobs are materialized by `DurableStreamMounter`

## Resource discovery

Cross-host resource discovery is a durable-stream concept centered on `resources:tenant-*`.

Relevant code and docs:

- Rust publisher: [crates/fireline-resources/src/publisher.rs](../../crates/fireline-resources/src/publisher.rs)
- Rust registry: [crates/fireline-resources/src/registry.rs](../../crates/fireline-resources/src/registry.rs)
- proposal: [docs/proposals/resource-discovery.md](../proposals/resource-discovery.md)

Important current state:

- the Rust side has `StreamResourcePublisher` and `StreamResourceRegistry`
- the TS client currently exposes resource refs, not a high-level discovery client

So from a TS app’s perspective today:

- you can ask the sandbox to mount a `streamBlob(...)`
- server-side discovery and publication live in Rust
