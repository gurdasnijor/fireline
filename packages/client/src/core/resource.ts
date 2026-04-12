/**
 * Serializable resource references for the managed-agent Resources primitive
 * defined in `docs/proposals/client-primitives.md` and mapped in
 * `docs/explorations/managed-agents-mapping.md`.
 *
 * Mirrors the Rust `ResourceSourceRef` + `PublishedResourceRef` split in
 * `crates/fireline-resources`.
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
