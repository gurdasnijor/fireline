import {
  createFirelineDB,
  type FirelineCollections,
  type FirelineDB as StateFirelineDB,
} from '@fireline/state'

const DEFAULT_STATE_STREAM_URL = 'http://localhost:7474/streams/state/default'

export interface FirelineDbOptions {
  readonly stateStreamUrl?: string
  readonly headers?: Record<string, string>
}

export type FirelineDB = StateFirelineDB & FirelineCollections

export async function db(options: FirelineDbOptions = {}): Promise<FirelineDB> {
  const stateStreamUrl =
    options.stateStreamUrl ??
    readFirelineStreamUrl() ??
    DEFAULT_STATE_STREAM_URL

  const rawDb = createFirelineDB({
    stateStreamUrl,
    headers: options.headers,
  })

  // Hoist collections onto the existing DB instance so callers get `db.sessions`
  // without changing the identity or lifecycle methods of the underlying StreamDB.
  const augmentedDb = rawDb as FirelineDB
  for (const [name, collection] of Object.entries(rawDb.collections)) {
    Object.defineProperty(augmentedDb, name, {
      configurable: true,
      enumerable: true,
      writable: false,
      value: collection,
    })
  }

  await augmentedDb.preload()
  return augmentedDb
}

function readFirelineStreamUrl(): string | undefined {
  if (typeof process === 'undefined') {
    return undefined
  }
  return process.env.FIRELINE_STREAM_URL
}
