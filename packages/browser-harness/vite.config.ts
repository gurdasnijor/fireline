import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'

export default defineConfig({
  plugins: [react()],
  server: {
    port: 5173,
    proxy: {
      '/api': {
        target: 'http://127.0.0.1:4436',
      },
      '/acp': {
        target: 'http://127.0.0.1:4437',
        ws: true,
      },
      '/v1': {
        target: 'http://127.0.0.1:4437',
      },
      '/healthz': {
        target: 'http://127.0.0.1:4437',
      },
    },
  },
})
