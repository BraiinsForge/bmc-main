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

// The web writes dates and figures with copies of the device's own tables in `bmc-shared`.
// The specs here read the Rust sources, so a change on either side fails instead of drifting apart.

import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, test } from '@rstest/core';

import * as pb from '@/proto';
import { formatBytes } from '@/lib/format';
import { dateLayout } from '@/lib/time';
import { store } from '@/store';

const rust = (path: string): string => readFileSync(resolve(process.cwd(), '..', path), 'utf8');
const timeRs = rust('bmc-shared/time/src/time.rs');
const numberRs = rust('bmc-shared/utils/src/number_format.rs');

/** `DdMmYyyyDot` → `DD_MM_YYYY_DOT`, the proto enum's spelling of the same variant. */
const protoName = (variant: string): string => (variant.match(/[A-Z][a-z]*/g) ?? []).join('_').toUpperCase();

/** The variants a proto enum names, its `UNSPECIFIED` aside, which no Rust variant mirrors. */
const protoVariants = (protoEnum: object): string[] =>
    Object.keys(protoEnum)
        .filter(key => Number.isNaN(Number(key)) && key !== 'UNSPECIFIED')
        .sort();

/** Every arm `pattern` finds, which must be one per variant of `protoEnum`, so a missed arm fails. */
function arms(source: string, pattern: RegExp, protoEnum: object): Array<[string, string]> {
    const found = [...source.matchAll(pattern)].map(([, variant, value]) => [variant, value] as [string, string]);
    expect(found.map(([variant]) => protoName(variant)).sort(), `the arms ${pattern} matched`).toEqual(
        protoVariants(protoEnum),
    );
    return found;
}

function defaultVariant(source: string, name: string): string {
    const found = source.match(new RegExp(`pub enum ${name} \\{[^}]*?#\\[default\\]\\s*(\\w+)`));
    expect(found, `no #[default] in ${name}`).not.toBeNull();
    return found?.[1] ?? '';
}

describe('the web copies the device formats', () => {
    test.each(arms(timeRs, /DateFormat::(\w+) => "([^"]+)"/g, pb.DateFormat))('date layout %s', (variant, layout) => {
        const format = pb.DateFormat[protoName(variant) as keyof typeof pb.DateFormat];
        expect(format, `${variant} has no proto value`).toBeDefined();
        expect(dateLayout(format)).toBe(layout);
    });

    test.each(arms(numberRs, /NumberFormat::(\w+) => \((?:NBSP|"[^"]*"), "([^"]*)"\)/g, pb.NumberFormat))(
        'decimal mark of %s',
        (variant, decimal) => {
            const format = pb.NumberFormat[protoName(variant) as keyof typeof pb.NumberFormat];
            expect(format, `${variant} has no proto value`).toBeDefined();
            expect(formatBytes(1_500, format)).toBe(`1${decimal}5 kB`);
        },
    );

    test('the defaults are the device’s', () => {
        const hours: Record<string, pb.TimeFormat> = {
            Hour12: pb.TimeFormat.TIME_FORMAT_12_HOUR,
            Hour24: pb.TimeFormat.TIME_FORMAT_24_HOUR,
        };
        expect(store.formats).toEqual({
            date: pb.DateFormat[protoName(defaultVariant(timeRs, 'DateFormat')) as keyof typeof pb.DateFormat],
            time: hours[defaultVariant(timeRs, 'TimeSystem')],
            number: pb.NumberFormat[
                protoName(defaultVariant(numberRs, 'NumberFormat')) as keyof typeof pb.NumberFormat
            ],
            firstDay: pb.Weekday[protoName(defaultVariant(timeRs, 'WeekDay')) as keyof typeof pb.Weekday],
        });
    });

    test('a format left unspecified is written as the device’s default', () => {
        const date = pb.DateFormat[protoName(defaultVariant(timeRs, 'DateFormat')) as keyof typeof pb.DateFormat];
        const number =
            pb.NumberFormat[protoName(defaultVariant(numberRs, 'NumberFormat')) as keyof typeof pb.NumberFormat];

        expect(dateLayout(pb.DateFormat.UNSPECIFIED)).toBe(dateLayout(date));
        expect(formatBytes(1_500, pb.NumberFormat.UNSPECIFIED)).toBe(formatBytes(1_500, number));
    });
});
