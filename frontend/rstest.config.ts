import { defineConfig, type RstestConfig } from '@rstest/core';
import rsbuildConfig from './rsbuild.config';

const { plugins, resolve } = rsbuildConfig;

export default defineConfig({
    include: ['**/*.{test,spec}.?(c|m)[jt]s?(x)'],
    exclude: ['**/node_modules/**'],
    globals: false,
    testEnvironment: 'jsdom',
    setupFiles: ['./rstest.setup.ts'],
    root: '.',

    plugins,
    resolve,
    // Same SWC transforms as the app build, but `@swc/plugin-formatjs` runs
    // WITHOUT `ast: true`: it injects the message `id` react-intl needs while
    // keeping `defaultMessage` a string. ast:true emits ICU AST objects, which
    // break components that render a raw defaultMessage under a message-less
    // test IntlProvider.
    tools: {
        swc: {
            jsc: {
                experimental: {
                    plugins: [
                        ['@swc/plugin-formatjs', {}],
                        ['@swc/plugin-emotion', {}],
                    ],
                },
            },
        },
    },
} as RstestConfig);
