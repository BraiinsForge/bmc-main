import path from 'node:path';
import url from 'node:url';

import { defineConfig } from 'vitest/config';
import tsconfigPaths from 'vite-tsconfig-paths';

const pathRoot = path.dirname(url.fileURLToPath(import.meta.url));

export default defineConfig({
    plugins: [tsconfigPaths()],
    test: {
        globals: true,
        environment: 'jsdom',
        setupFiles: path.resolve(pathRoot, 'vitest-setup.ts'),
        globalSetup: path.resolve(pathRoot, 'vitest-globals.ts'),
    },
});
