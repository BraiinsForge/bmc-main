/** @type {import('svgo').Config} */
export default {
    multipass: true,
    js2svg: { pretty: true, indent: 4 },
    plugins: [
        {
            name: 'preset-default',
            params: {
                overrides: {
                    // Preserve `id` attributes (preset-default's
                    // `cleanupIds` would otherwise drop unused ones).
                    // Filter SVGs reference their own ids via `url(#…)`
                    // and asset filenames sometimes carry the original
                    // id as a stable handle — keeping ids avoids those
                    // silent breakages.
                    cleanupIds: false,
                },
            },
        },
        // Explicitly remove <title> elements (not in preset-default in this svgo version).
        'removeTitle',
        // Drop elements that are invisible: fill:none (or no fill) and no stroke.
        // Catches leftover bounding-box rects from design tools.
        {
            name: 'removeInvisibleElements',
            fn: () => ({
                element: {
                    enter: (node, parentNode) => {
                        if (
                            node.name !== 'path' &&
                            node.name !== 'rect' &&
                            node.name !== 'circle' &&
                            node.name !== 'ellipse' &&
                            node.name !== 'polygon' &&
                            node.name !== 'polyline' &&
                            node.name !== 'line'
                        ) {
                            return;
                        }
                        const style = node.attributes.style || '';
                        const fill = node.attributes.fill || '';
                        const stroke = node.attributes.stroke || '';
                        const hasFillNone =
                            fill === 'none' || style.includes('fill:none') || style.includes('fill: none');
                        const hasStroke =
                            (stroke && stroke !== 'none') ||
                            (style.includes('stroke:') &&
                                !style.includes('stroke:none') &&
                                !style.includes('stroke: none'));
                        // Only remove if explicitly fill:none and no stroke
                        if (hasFillNone && !hasStroke) {
                            parentNode.children = parentNode.children.filter(c => c !== node);
                        }
                    },
                },
            }),
        },
    ],
};
