/** @type {import('svgo').Config} */
export default {
    multipass: true,
    js2svg: { pretty: true, indent: 4 },
    plugins: [
        {
            name: 'preset-default',
            params: {
                overrides: {
                    // disable a default plugin
                    cleanupIds: false,

                    // customize the params of a default plugin
                    inlineStyles: { onlyMatchedOnce: false },
                },
            },
        },
    ],
};
