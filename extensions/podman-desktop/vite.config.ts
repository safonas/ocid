import { builtinModules } from 'node:module';
import { defineConfig } from 'vite';

export default defineConfig({
  build: {
    target: 'esnext',
    outDir: 'dist',
    minify: false,
    lib: {
      entry: 'src/extension.ts',
      formats: ['cjs'],
    },
    rollupOptions: {
      external: ['@podman-desktop/api', ...builtinModules.flatMap(m => [m, `node:${m}`])],
      output: {
        entryFileNames: '[name].cjs',
      },
    },
    emptyOutDir: true,
    reportCompressedSize: false,
  },
});
