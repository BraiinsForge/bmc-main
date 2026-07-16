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
