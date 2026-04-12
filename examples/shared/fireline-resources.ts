import type { ResourceRef } from '../../packages/client/src/resources.ts'

export function localPath(path: string, mountPath: string, readOnly = false): ResourceRef {
  return {
    source_ref: { kind: 'localPath', host_id: 'local', path },
    mount_path: mountPath,
    read_only: readOnly,
  }
}

export function streamBlob(stream: string, key: string, mountPath: string): ResourceRef {
  return {
    source_ref: { kind: 'durableStreamBlob', stream, key },
    mount_path: mountPath,
    read_only: false,
  }
}

export function gitRepo(url: string, ref: string, mountPath: string): ResourceRef {
  return {
    source_ref: { kind: 'gitRepo', url, ref, path: '/' },
    mount_path: mountPath,
    read_only: false,
  }
}

export function ociImage(image: string, path: string, mountPath: string): ResourceRef {
  return {
    source_ref: { kind: 'ociImageLayer', image, path },
    mount_path: mountPath,
    read_only: true,
  }
}

export function httpUrl(url: string, mountPath: string): ResourceRef {
  return {
    source_ref: { kind: 'httpUrl', url },
    mount_path: mountPath,
    read_only: true,
  }
}
