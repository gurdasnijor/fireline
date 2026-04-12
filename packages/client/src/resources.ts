/**
 * Serializable resource references for the managed-agent Resources primitive
 * defined in `docs/proposals/client-api-redesign.md`.
 */
export type HostId = string

export type StreamFsMode = 'snapshotReadOnly' | 'liveReadOnly' | 'liveReadWrite'

export type ResourceSourceRef =
  | {
      readonly kind: 'localPath'
      readonly host_id: HostId
      readonly path: string
    }
  | {
      readonly kind: 's3'
      readonly bucket: string
      readonly key: string
      readonly region: string
      readonly endpoint_url?: string
    }
  | {
      readonly kind: 'gcs'
      readonly bucket: string
      readonly key: string
    }
  | {
      readonly kind: 'dockerVolume'
      readonly host_id: HostId
      readonly volume_name: string
      readonly path_within_volume: string
    }
  | {
      readonly kind: 'durableStreamBlob'
      readonly stream: string
      readonly key: string
    }
  | {
      readonly kind: 'streamFs'
      readonly source_ref: string
      readonly revision?: string
      readonly mode: StreamFsMode
    }
  | {
      readonly kind: 'ociImageLayer'
      readonly image: string
      readonly path: string
    }
  | {
      readonly kind: 'gitRepo'
      readonly url: string
      readonly ref: string
      readonly path: string
    }
  | {
      readonly kind: 'httpUrl'
      readonly url: string
      readonly headers?: Readonly<Record<string, string>>
    }

export type PublishedResourceRef = {
  readonly source_ref: ResourceSourceRef
  readonly mount_path: string
  readonly read_only?: boolean
}

export type ResourceRef = PublishedResourceRef

/**
 * Creates a local-path resource ref mounted into the sandbox.
 */
export function localPath(path: string, mountPath: string, readOnly = false): ResourceRef {
  return {
    source_ref: { kind: 'localPath', host_id: 'local', path },
    mount_path: mountPath,
    ...(readOnly ? { read_only: true } : {}),
  }
}

/**
 * Creates a durable-stream blob resource ref mounted into the sandbox.
 */
export function streamBlob(stream: string, key: string, mountPath: string): ResourceRef {
  return {
    source_ref: { kind: 'durableStreamBlob', stream, key },
    mount_path: mountPath,
    read_only: true,
  }
}

/**
 * Creates a git repository resource ref mounted into the sandbox.
 */
export function gitRepo(url: string, ref: string, mountPath: string): ResourceRef {
  return {
    source_ref: { kind: 'gitRepo', url, ref, path: '/' },
    mount_path: mountPath,
    read_only: false,
  }
}

/**
 * Creates an OCI image layer resource ref mounted into the sandbox.
 */
export function ociImage(image: string, path: string, mountPath: string): ResourceRef {
  return {
    source_ref: { kind: 'ociImageLayer', image, path },
    mount_path: mountPath,
    read_only: true,
  }
}

/**
 * Creates an HTTP resource ref mounted into the sandbox.
 */
export function httpUrl(url: string, mountPath: string): ResourceRef {
  return {
    source_ref: { kind: 'httpUrl', url },
    mount_path: mountPath,
    read_only: true,
  }
}
