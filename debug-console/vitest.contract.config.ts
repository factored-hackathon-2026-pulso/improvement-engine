import { defineConfig } from 'vitest/config';
export default defineConfig({
  test: { include: ['tests/contract/**/*.test.ts'], environment: 'node', pool: 'threads', testTimeout: 15000, hookTimeout: 20000 },
});
