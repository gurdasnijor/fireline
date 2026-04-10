import { zodToJsonSchema } from 'zod-to-json-schema'

import {
  chunkSchema,
  connectionSchema,
  pendingRequestSchema,
  permissionSchema,
  promptTurnSchema,
  runtimeInstanceSchema,
  terminalSchema,
} from '../src/schema.js'

type CollectionSpec = {
  entityType: string
  primaryKey: string
  schema: unknown
}

const collections: Record<string, CollectionSpec> = {
  connections: {
    entityType: 'connection',
    primaryKey: 'logicalConnectionId',
    schema: zodToJsonSchema(connectionSchema, {
      name: 'ConnectionRow',
      $refStrategy: 'none',
    }),
  },
  promptTurns: {
    entityType: 'prompt_turn',
    primaryKey: 'promptTurnId',
    schema: zodToJsonSchema(promptTurnSchema, {
      name: 'PromptTurnRow',
      $refStrategy: 'none',
    }),
  },
  pendingRequests: {
    entityType: 'pending_request',
    primaryKey: 'requestId',
    schema: zodToJsonSchema(pendingRequestSchema, {
      name: 'PendingRequestRow',
      $refStrategy: 'none',
    }),
  },
  permissions: {
    entityType: 'permission',
    primaryKey: 'requestId',
    schema: zodToJsonSchema(permissionSchema, {
      name: 'PermissionRow',
      $refStrategy: 'none',
    }),
  },
  terminals: {
    entityType: 'terminal',
    primaryKey: 'terminalId',
    schema: zodToJsonSchema(terminalSchema, {
      name: 'TerminalRow',
      $refStrategy: 'none',
    }),
  },
  runtimeInstances: {
    entityType: 'runtime_instance',
    primaryKey: 'instanceId',
    schema: zodToJsonSchema(runtimeInstanceSchema, {
      name: 'RuntimeInstanceRow',
      $refStrategy: 'none',
    }),
  },
  chunks: {
    entityType: 'chunk',
    primaryKey: 'chunkId',
    schema: zodToJsonSchema(chunkSchema, {
      name: 'ChunkRow',
      $refStrategy: 'none',
    }),
  },
}

const stateProtocol = {
  $schema: 'https://json-schema.org/draft/2020-12/schema',
  title: 'Fireline State Protocol',
  description: 'STATE-PROTOCOL change messages emitted by Fireline.',
  oneOf: Object.values(collections).map((collection) => ({
    type: 'object',
    properties: {
      type: { const: collection.entityType },
      key: { type: 'string' },
      value: collection.schema,
      headers: {
        type: 'object',
        properties: {
          operation: {
            type: 'string',
            enum: ['insert', 'update', 'delete'],
          },
        },
        required: ['operation'],
        additionalProperties: true,
      },
    },
    required: ['type', 'key', 'headers'],
    additionalProperties: false,
  })),
}

process.stdout.write(
  JSON.stringify(
    {
      $schema: 'https://json-schema.org/draft/2020-12/schema',
      title: 'Fireline State Schemas',
      collections,
      stateProtocol,
    },
    null,
    2,
  ),
)
