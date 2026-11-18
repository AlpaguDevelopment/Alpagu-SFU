import { defineConfig } from 'vite';
import basicSsl from '@vitejs/plugin-basic-ssl';

// Phones connect over LAN; getUserMedia requires a secure context (HTTPS).
// Accept the self-signed certificate warning once in the browser.
// Proxy same-origin /ws signaling to core (docs/protocol.md).
const CORE = process.env.SFU_CORE ?? 'ws://127.0.0.1:7880';

export default defineConfig({
  plugins: [basicSsl()],
  server: {
    host: true,
    port: 5173,
    strictPort: true,
    proxy: {
      '/ws': { target: CORE, ws: true, changeOrigin: true },
    },
  },
});
