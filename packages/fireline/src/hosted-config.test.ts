import assert from 'node:assert/strict'
import test from 'node:test'
import {
  parseHostedConfig,
  resolveHostedDeploy,
  targetTokenEnvVar,
  type HostedConfig,
} from './hosted-config.js'

test('parseHostedConfig accepts camel and kebab aliases for hosted target fields', () => {
  const config = parseHostedConfig(
    JSON.stringify({
      defaultTarget: 'production',
      targets: {
        production: {
          provider: 'fly',
          region: 'lax',
          'resource-naming': {
            'app-name': 'reviewer-prod',
          },
          'auth-ref': 'FLY_API_TOKEN',
        },
      },
    }),
  )

  assert.equal(config.defaultTarget, 'production')
  assert.deepEqual(config.targets.production, {
    provider: 'fly',
    region: 'lax',
    resourceNaming: { appName: 'reviewer-prod' },
    authRef: 'FLY_API_TOKEN',
  })
})

test('resolveHostedDeploy uses default target provider and target-specific env fallback', () => {
  const config: HostedConfig = {
    defaultTarget: 'production',
    targets: {
      production: {
        provider: 'fly',
      },
    },
  }

  const resolved = resolveHostedDeploy({
    config,
    targetName: null,
    deployTarget: null,
    token: null,
    env: {
      [targetTokenEnvVar('production')]: 'prod-token',
    },
  })

  assert.equal(resolved.targetName, 'production')
  assert.equal(resolved.deployTarget, 'fly')
  assert.equal(resolved.token, 'prod-token')
  assert.equal(resolved.tokenSource, `env:${targetTokenEnvVar('production')}`)
  assert.equal(resolved.tokenSinkEnvVar, 'FLY_API_TOKEN')
})

test('resolveHostedDeploy lets explicit token override config and env lookup', () => {
  const resolved = resolveHostedDeploy({
    config: {
      defaultTarget: 'edge',
      targets: {
        edge: {
          provider: 'cloudflare-containers',
          authRef: 'CLOUDFLARE_API_TOKEN',
        },
      },
    },
    targetName: 'edge',
    deployTarget: null,
    token: 'cli-token',
    env: {
      CLOUDFLARE_API_TOKEN: 'env-token',
    },
  })

  assert.equal(resolved.token, 'cli-token')
  assert.equal(resolved.tokenSource, '--token')
  assert.equal(resolved.tokenSinkEnvVar, 'CLOUDFLARE_API_TOKEN')
})

test('resolveHostedDeploy rejects mismatched --target and --to combinations', () => {
  assert.throws(
    () =>
      resolveHostedDeploy({
        config: {
          defaultTarget: null,
          targets: {
            staging: {
              provider: 'fly',
            },
          },
        },
        targetName: 'staging',
        deployTarget: 'k8s',
        token: null,
        env: {},
      }),
    /conflicts with --to k8s/,
  )
})
