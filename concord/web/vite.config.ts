import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import tailwindcss from '@tailwindcss/vite'

export default defineConfig({
  plugins: [react(), tailwindcss()],
  build: {
    sourcemap: true,
  },
  server: {
    port: 3000,
    proxy: {
      '/api': process.env.CONCORD_BACKEND_URL ?? 'http://localhost:8080',
      '/oauth': process.env.CONCORD_BACKEND_URL ?? 'http://localhost:8080',
      '/ws': {
        target: (process.env.CONCORD_BACKEND_URL ?? 'http://localhost:8080').replace(/^http/, 'ws'),
        ws: true,
      },
    },
  },
})
