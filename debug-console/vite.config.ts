import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';
const target = 'http://127.0.0.1:4010';
export default defineConfig({
  plugins: [react()],
  server: { port: 5173, proxy: { '/internal': target, '/api': target, '/__fixture': target } },
  test: { include: ['tests/unit/**/*.test.ts', 'tests/component/**/*.test.tsx'], environment: 'jsdom', pool: 'threads' },
});
