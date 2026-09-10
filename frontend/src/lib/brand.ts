// Copyright (C) 2026  Braiins Systems s.r.o.
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

import type { CSSProperties } from 'react';
import { isPlainObject } from 'es-toolkit';

// Boser's `/var/brand.js` assigns `window.BRAND`; BMC appends it to `/system.js` on managed devices.
// Parsing mirrors boser's `BRAND.tsx`: a bad field is null, the rest survive.

export interface BrandLink {
    href: string;
    text: string;
    isActive?: boolean;
}
export interface BrandImage {
    src: string;
    style?: CSSProperties;
}
export interface Brand {
    name: null | string;
    logo: { header: null | BrandImage };
    links: { products: null | BrandLink[] };
}

function isStringNE(v: unknown): v is string {
    return typeof v === 'string' && v.length > 0;
}
function isLink(v: unknown): v is BrandLink {
    return (
        isPlainObject(v) &&
        isStringNE(v.href) &&
        isStringNE(v.text) &&
        (v.isActive == null || typeof v.isActive === 'boolean')
    );
}
function field(obj: unknown, key: string): unknown {
    return isPlainObject(obj) ? obj[key] : undefined;
}
function getString(v: unknown): null | string {
    return isStringNE(v) ? v : null;
}
// Only the sizing and spacing a logo needs; anything else could reposition the header.
const LOGO_STYLE_KEYS = ['padding', 'margin', 'width', 'height', 'maxWidth', 'maxHeight'] as const;
function getStyle(v: unknown): undefined | CSSProperties {
    if (!isPlainObject(v)) return undefined;
    const style: CSSProperties = {};
    for (const key of LOGO_STYLE_KEYS) {
        const value = v[key];
        if (typeof value === 'number' || isStringNE(value)) style[key] = value;
    }
    return style;
}
function getImage(v: unknown): null | BrandImage {
    if (!isPlainObject(v) || !isStringNE(v.src)) return null;
    return { src: v.src, style: getStyle(v.style) };
}
function getLinks(v: unknown): null | BrandLink[] {
    return Array.isArray(v) && v.every(isLink) ? v : null;
}

export function parseBrand(raw: unknown): Brand {
    return {
        name: getString(field(raw, 'name')),
        logo: { header: getImage(field(field(raw, 'logo'), 'header')) },
        links: { products: getLinks(field(field(raw, 'links'), 'products')) },
    };
}

/** Reads the global the blocking `system.js` script in index.html assigns before the bundle runs. */
export function readBrand(): Brand {
    return parseBrand((window as { BRAND?: unknown }).BRAND);
}
