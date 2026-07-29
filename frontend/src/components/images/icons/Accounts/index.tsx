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

import { Password as IconGeneric } from '@carbon/react/icons';
import type * as pb from '@/proto';

export interface AccountIconProps {
    // Artwork the backend may supply with the credential type
    icon?: pb.Icon;
    size: number;

    className?: string;
    style?: CSSProperties;
}

// A type ships its own artwork, so a new one needs no change here.
// The fallback is a state the backend declares, not one this component guesses at:
// the key means "a credential", never a particular type,
// so it cannot render as the wrong one.
//
// Decorative — every caller puts the type or account name beside it.
export function AccountIcon({ icon, size, style, className }: AccountIconProps) {
    return !icon ? (
        <IconGeneric size={size} style={style} className={className} />
    ) : (
        <img
            src={`data:${icon.mimeType};base64,${icon.data}`}
            alt=""
            width={size}
            height={size}
            style={style}
            className={className}
        />
    );
}
