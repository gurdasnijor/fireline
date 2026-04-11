/**
 * Serializable resource references for the managed-agent Resources primitive
 * defined in `docs/proposals/client-primitives.md` and mapped in
 * `docs/explorations/managed-agents-mapping.md`.
 */
export type ResourceRef =
  | {
      readonly kind: 'local_path'
      readonly path: string
      readonly mount_path: string
      readonly read_only?: boolean
    }
  | {
      readonly kind: 'git_remote'
      readonly repo_url: string
      readonly ref?: string
      readonly subdir?: string
      readonly mount_path: string
      readonly read_only?: boolean
    }
  | { readonly kind: 's3'; readonly bucket: string; readonly prefix: string; readonly mount_path: string }
  | {
      readonly kind: 'gcs'
      readonly bucket: string
      readonly prefix: string
      readonly mount_path: string
    }
