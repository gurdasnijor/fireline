// Fireline
import type { ResourceRef } from '@fireline/client/resources'

export function localPathResource(
  path: string,
  mountPath = '/workspace',
  readOnly = true,
): ResourceRef {
  return {
    source_ref: { kind: 'localPath', host_id: 'local', path },
    mount_path: mountPath,
    read_only: readOnly,
  }
}
