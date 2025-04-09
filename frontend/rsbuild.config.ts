import { defineConfig } from '@rsbuild/core';
import { pluginReact } from '@rsbuild/plugin-react';
import { pluginSass } from '@rsbuild/plugin-sass';
import { pluginSvgr } from '@rsbuild/plugin-svgr';
import { pluginTypedCSSModules } from '@rsbuild/plugin-typed-css-modules';

const isProduction = process.env.NODE_ENV === 'production';

export default defineConfig({
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
        favicon: './src/res/svg/ii.svg',
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
        },
    },

    tools: {
        lightningcssLoader: {
            errorRecovery: true,
        },

        rspack: {
            devtool: 'source-map',
            experiments: {
                css: true,
                topLevelAwait: true,
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
                    plugins: [['@swc/plugin-formatjs', { ast: true }]],
                },
            },
        },
    },

    server: {
        open: false,
        compress: true,
        printUrls: true,
        proxy: {
            '/braiins.bmc': {
                target: 'http://localhost:6070',
                changeOrigin: true,
            },
        },
    },
});
