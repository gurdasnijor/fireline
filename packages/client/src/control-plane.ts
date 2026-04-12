export interface ControlPlaneClientOptions {
  readonly serverUrl: string
  readonly token?: string
}

export interface ControlPlaneRequestOptions {
  readonly method?: string
  readonly body?: string
  readonly allowNotFound?: boolean
}

export async function requestControlPlane<T>(
  client: ControlPlaneClientOptions,
  path: string,
  options: ControlPlaneRequestOptions = {},
): Promise<T | null> {
  const response = await fetch(`${normalizeServerUrl(client.serverUrl)}${path}`, {
    method: options.method ?? 'GET',
    headers: {
      accept: 'application/json',
      ...(options.body ? { 'content-type': 'application/json' } : {}),
      ...(client.token ? { authorization: `Bearer ${client.token}` } : {}),
    },
    body: options.body,
  })

  if (response.status === 404 && options.allowNotFound) {
    return null
  }

  if (!response.ok) {
    const error = new Error(`${response.status} ${response.statusText}: ${await readControlPlaneError(response)}`)
    throw Object.assign(error, { status: response.status })
  }

  return (await response.json()) as T
}

export function normalizeServerUrl(serverUrl: string): string {
  return serverUrl.replace(/\/$/, '')
}

async function readControlPlaneError(response: Response): Promise<string> {
  try {
    const payload = (await response.json()) as { readonly error?: string }
    return payload.error ?? response.statusText
  } catch {
    return response.statusText
  }
}
