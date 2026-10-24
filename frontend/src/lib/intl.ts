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

import { isPlainObject } from 'es-toolkit';
import { createIntl, createIntlCache, type IntlConfig, type IntlShape } from 'react-intl';

/** The language every message's source is written in. */
export const SOURCE_LOCALE = 'en';

/** The language the web UI speaks, by BCP-47 tag, with its catalog already loaded. */
export interface Locale {
    tag: string;
    messages?: IntlConfig['messages'];
}

/**
 * Whether `tag` is an English, which the web's source is written in, whatever its region.
 */
function isEnglish(tag: string): boolean {
    return tag === SOURCE_LOCALE || tag.startsWith(`${SOURCE_LOCALE}-`);
}

const cache = createIntlCache();

/**
 * An English is its own default locale: a message its catalog lacks falls back to the source
 * without reading as missing, and the source's arguments are formatted in that English.
 */
export function intlFor({ tag, messages }: Locale): IntlShape {
    const defaultLocale = isEnglish(tag) ? tag : SOURCE_LOCALE;
    return createIntl({ locale: tag, defaultLocale, messages }, cache);
}

/**
 * The language the device boots the web in, with its catalog,
 * from the `/language.js` it serves. `null` boots in English:
 * a device without its languages sends it, and so does one
 * whose setting it no longer offers.
 */
export function bootLocale(): Locale | null {
    const raw = (window as { LANGUAGE?: unknown }).LANGUAGE;
    if (!isPlainObject(raw) || typeof raw.tag !== 'string') return null;
    return { tag: raw.tag, messages: isPlainObject(raw.messages) ? (raw.messages as Locale['messages']) : undefined };
}
