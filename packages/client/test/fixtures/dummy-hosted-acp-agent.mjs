import { randomUUID } from 'node:crypto'
import { createServer } from 'node:http'
import { fileURLToPath } from 'node:url'

export async function start() {
  const runtimes = new Map()
  const timers = new Set()
  const server = createServer(async (req, res) => {
    const url = new URL(req.url ?? '/', 'http://127.0.0.1')
    const send = (status, body) => {
      res.writeHead(status, { 'content-type': 'application/json' })
      res.end(JSON.stringify(body))
    }
    if (req.method === 'POST' && url.pathname === '/v1/runtimes') {
      const handle_id = randomUUID()
      const runtime = {
        handle_id,
        status: 'starting',
        acp: { url: `http://127.0.0.1/runtimes/${handle_id}/acp` },
        state: { url: `http://127.0.0.1/runtimes/${handle_id}/state` },
      }
      runtimes.set(handle_id, runtime)
      const timer = setTimeout(() => {
        if (runtime.status === 'starting') runtime.status = 'ready'
        timers.delete(timer)
      }, 25)
      timers.add(timer)
      return send(200, runtime)
    }
    const match = url.pathname.match(/^\/v1\/runtimes\/([^/]+)(?:\/(wake|stop))?$/)
    if (!match) return send(404, { error: 'not_found' })
    const runtime = runtimes.get(decodeURIComponent(match[1]))
    if (!runtime) return send(404, { error: 'not_found' })
    if (req.method === 'GET' && !match[2]) return send(200, runtime)
    if (req.method === 'POST' && match[2] === 'wake') {
      return send(200, runtime.status === 'ready' ? { outcome: 'noop' } : { outcome: 'advanced', steps: 1 })
    }
    if (req.method === 'POST' && match[2] === 'stop') {
      runtime.status = 'stopped'
      return send(200, runtime)
    }
    return send(405, { error: 'method_not_allowed' })
  })

  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve))
  const address = server.address()
  if (!address || typeof address === 'string') throw new Error('dummy hosted ACP agent failed to bind')

  return {
    url: `http://127.0.0.1:${address.port}`,
    stop: async () => {
      for (const timer of timers) clearTimeout(timer)
      timers.clear()
      await new Promise((resolve, reject) => server.close((error) => (error ? reject(error) : resolve())))
    },
  }
}

if (process.argv[1] && fileURLToPath(import.meta.url) === process.argv[1]) {
  const { url } = await start()
  process.stdout.write(`${url}\n`)
}
