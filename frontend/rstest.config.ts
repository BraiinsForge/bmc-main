import { defineConfig } from '@rstest/core';
import rsbuildConfig from './rsbuild.config';

const { plugins, resolve } = rsbuildConfig;

export default defineConfig({
    include: ['**/*.{test,spec}.?(c|m)[jt]s?(x)'],
    exclude: ['**/node_modules/**'],
    globals: false,
    testEnvironment: 'jsdom',
    root: '.',

    plugins,
    resolve,
});
