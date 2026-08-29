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

import { defineConfig, type ProxyOptions } from '@rsbuild/core';
import { pluginReact } from '@rsbuild/plugin-react';
import { pluginSass } from '@rsbuild/plugin-sass';
import { pluginSvgr } from '@rsbuild/plugin-svgr';
import { pluginTypedCSSModules } from '@rsbuild/plugin-typed-css-modules';

import SVGO_CONFIG from './svgo.config.js';
import SVGR_TEMPLATE from './svgr.template.js';

const isProduction = process.env.NODE_ENV === 'production';
const proxyConf: ProxyOptions = {
    target: process.env.BMC_BACKEND || 'http://localhost:6070',
    changeOrigin: true,
    followRedirects: true,
    // Without this the abort signals do not propagate to the server
    // which we really need, because for example the play sound RPC
    // needs to stop the playback when the connection is dropped.
    on: {
        proxyReq(proxyReq, _, res) {
            res.on('close', () => proxyReq.destroy());
        },
    },
};

export default defineConfig({
    source: {
        entry: {
            index: { html: true, import: './src/index-app.tsx' },
            'index-connect': { html: true, import: './src/index-init.tsx' },
        },
    },

    mode: isProduction ? 'production' : 'development',

    resolve: {
        extensions: ['...', '.ts', '.tsx', '.jsx'],
        alias: {
            styles: './src/styles',
        },
    },

    plugins: [
        pluginReact({
            fastRefresh: true,
            enableProfiler: false,
            swcReactOptions: { runtime: 'automatic' },
        }),
        pluginSvgr({
            svgrOptions: {
                exportType: 'default',
                template: SVGR_TEMPLATE,
                svgoConfig: SVGO_CONFIG,
            },
        }),
        pluginSass(),
        pluginTypedCSSModules(),
    ],

    dev: {
        hmr: true,
        liveReload: true,
        progressBar: true,
        writeToDisk: true,
    },

    html: {
        template: './src/index.html',
        favicon: './src/icon.png',
    },

    output: {
        emitAssets: true,
        sourceMap: !isProduction,
        legalComments: 'linked',

        emitCss: true,
        cssModules: {
            // Math all scss files that do not have `.global` in their name
            auto: resource => {
                return resource.includes('.scss') && !resource.includes('.global.');
            },
            mode: 'local',
            namedExport: false,
            exportGlobals: false,
            exportLocalsConvention: 'asIs',
            localIdentName: isProduction ? '[local]-[hash:base64:6]' : '[name]__[local]-[hash:base64:6]',
        },
    },

    tools: {
        lightningcssLoader: { errorRecovery: true },

        rspack: {
            experiments: {
                css: true,
                futureDefaults: true,
            },
            module: {
                rules: [
                    {
                        test: /\.(png|jpg)$/,
                        issuer: /\.[jt]sx?$/,
                        type: 'asset/resource',
                    },
                ],
            },
        },

        swc: {
            jsc: {
                experimental: {
                    plugins: [
                        ['@swc/plugin-formatjs', { ast: true }],
                        ['@swc/plugin-emotion', {}],
                    ],
                },
            },
        },
    },

    server: {
        open: false,
        compress: true,
        printUrls: true,
        proxy: {
            // gRPC-web api endpoints
            '/braiins.bmc': proxyConf,
            // REST API endpoints
            '/api': proxyConf,
            // Widget icons served by the BMC HTTP server
            '/widgets': proxyConf,
        },
    },
});
