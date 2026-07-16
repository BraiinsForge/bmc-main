// Copyright (C) 2025  Braiins Systems s.r.o.
// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

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
