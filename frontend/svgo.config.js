/** @type {import('svgo').Config} */
export default {
    multipass: true,
    js2svg: { pretty: true },
    plugins: [
        {
            name: 'preset-default',
            params: {
                overrides: {
                    // disable a default plugin
                    cleanupIds: false,
                    // viewBox is required to resize SVGs with CSS.
                    // @see https://github.com/svg/svgo/issues/1128
                    removeViewBox: false,

                    // customize the params of a default plugin
                    inlineStyles: {
                        onlyMatchedOnce: false,
                    },
                },
            },
        },
    ],
};
