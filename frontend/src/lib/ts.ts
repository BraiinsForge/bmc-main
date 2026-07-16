// Copyright (C) 2025  Braiins Systems s.r.o.
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

/**
 * Used for exhaustive switch checking
 * @see https://stackoverflow.com/questions/39419170/how-do-i-check-that-a-switch-block-is-exhaustive-in-typescript
 */
export function assertUnreachable(x: never, label?: string): never {
    const message = label ? `Unexected ${label} value has been reached!` : 'Unexected value has been reached!';
    throw new Error(`${message} - ${JSON.stringify(x)}`);
}

/**
 * Used for exhaustive switch checking
 * @see https://stackoverflow.com/questions/39419170/how-do-i-check-that-a-switch-block-is-exhaustive-in-typescript
 */
export function assertUndefined(x: undefined, label?: string) {
    const message = label ? `Unexected ${label} value has been reached!` : 'Unexected value has been reached!';
    if (typeof x !== 'undefined') throw new Error(`${message} - ${JSON.stringify(x)}`);
}
