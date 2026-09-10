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

import { describe, expect, test } from '@rstest/core';
import { parseBrand } from './brand';

const VALID = {
    name: 'Acme OS',
    logo: { header: { src: '/var/header.svg', style: { padding: 14 } } },
    links: { products: [{ href: 'https://acme.example', text: 'Acme', isActive: true }] },
};

describe('parseBrand', () => {
    test('takes every field from a valid brand file', () => {
        expect(parseBrand(VALID)).toEqual(VALID);
    });

    test('yields no fields when the global is missing', () => {
        expect(parseBrand(undefined)).toEqual({ name: null, logo: { header: null }, links: { products: null } });
    });

    test('keeps the valid fields when one is malformed', () => {
        const brand = parseBrand({ ...VALID, links: { products: [{ href: 'https://acme.example' }] } });

        expect(brand.logo.header).toEqual(VALID.logo.header);
        expect(brand.links.products).toBeNull();
    });

    test('drops a logo without a source', () => {
        expect(parseBrand({ ...VALID, logo: { header: { style: { padding: 14 } } } }).logo.header).toBeNull();
    });

    test('keeps only the sizing and spacing of a logo style', () => {
        const style = { padding: 14, width: '4rem', position: 'fixed', height: { px: 1 } };

        expect(parseBrand({ ...VALID, logo: { header: { src: '/var/header.svg', style } } }).logo.header).toEqual({
            src: '/var/header.svg',
            style: { padding: 14, width: '4rem' },
        });
    });
});
