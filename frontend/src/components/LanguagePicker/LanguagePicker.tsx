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

import { useIntl } from 'react-intl';
import type { DropdownProps } from '@carbon/react';

import type * as pb from '@/proto';
import type { iField } from '@/lib/form';

import { Dropdown } from '../Dropdown';
import { LanguageName, LanguageUnavailable } from '../LanguageName';

export interface LanguagePickerProps {
    id: string;
    field: iField<string> & {
        items: ReadonlyArray<pb.LanguageInfo>;
        // The device is set to a language it no longer offers, and so sends none.
        unavailable?: boolean;
    };
}

const toString = (x: null | pb.LanguageInfo): string => x?.nativeName ?? '';
const renderElement = (x: null | pb.LanguageInfo): NonNullable<ReactNode> => (x ? <LanguageName language={x} /> : '');

/** The device's language, picked from those it offers. */
export function LanguagePicker({ id, field }: LanguagePickerProps) {
    const { formatMessage } = useIntl();
    const { value, items, unavailable, disabled, error, onChange } = field;
    const change: DropdownProps<null | pb.LanguageInfo>['onChange'] = x => {
        if (x.selectedItem) onChange?.(x.selectedItem.tag);
    };

    return (
        <Dropdown<null | pb.LanguageInfo>
            id={id}
            size="md"
            label={unavailable ? <LanguageUnavailable /> : ''}
            titleText={formatMessage({
                defaultMessage: 'Language',
                description: "Accessible name of the settings' language dropdown, read out by screen readers",
            })}
            hideLabel
            items={Array.from(items)}
            selectedItem={items.find(x => x.tag === value) ?? null}
            onChange={change}
            itemToString={toString}
            itemToElement={renderElement}
            renderSelectedItem={renderElement}
            disabled={disabled}
            invalid={!!error}
            invalidText={error}
        />
    );
}
