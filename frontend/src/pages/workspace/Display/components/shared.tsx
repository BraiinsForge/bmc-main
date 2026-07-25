// Copyright (C) 2025  Braiins Systems s.r.o.
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
import { FormattedMessage } from 'react-intl';

// App
import * as pb from '@/proto';
import type { iField } from '@/lib/form';
import type { OptionItem } from '@/components/ParamField';

// Components
import { ButtonSwitch, Checkbox } from '@/components';
import { Screen as IconScreen, Information as IconInfo } from '@carbon/react/icons';
import { RadioButtonGroup, RadioButton, Dropdown } from '@carbon/react';

// Styles
import css from './shared.scss';

export interface WidgetSizeSelectorProps {
    field: null | (iField<pb.WidgetSize> & { options: Array<Exclude<pb.WidgetSize, 0>> });
}
export function WidgetSizeSelector(props: WidgetSizeSelectorProps) {
    const intl = useIntl();
    const { field } = props;

    if (field == null) return null;
    const { formatMessage } = intl;
    const { options, value, error, onChange, disabled } = field;

    return (
        <ButtonSwitch
            options={[
                {
                    id: pb.WidgetSize.SMALL,
                    text: formatMessage({ defaultMessage: 'Small' }),
                    disabled: !options.includes(pb.WidgetSize.SMALL),
                },
                {
                    id: pb.WidgetSize.MEDIUM,
                    text: formatMessage({ defaultMessage: 'Medium' }),
                    disabled: !options.includes(pb.WidgetSize.MEDIUM),
                },
                {
                    id: pb.WidgetSize.LARGE,
                    text: formatMessage({ defaultMessage: 'Large' }),
                    disabled: !options.includes(pb.WidgetSize.LARGE),
                },
            ]}
            onChange={onChange}
            selectedOption={value}
            disabled={disabled}
            invalid={!!error}
            invalidText={error}
        />
    );
}

export interface BoundCheckboxProps extends iField<boolean> {
    id: string;
    labelText: string;
}
export function BoundCheckbox(props: BoundCheckboxProps) {
    const { id, value, labelText, error, onChange, disabled } = props;
    return (
        <Checkbox
            id={id}
            checked={!!value}
            label={labelText}
            disabled={disabled}
            onChange={(_, { checked }) => onChange?.(checked)}
            invalid={!!error}
            invalidText={error}
        />
    );
}

export interface BoundDropdownProps<T> extends iField<T> {
    id: string;
    labelText: string;
    placeholderText: string;
    decorator?: ReactNode;
    helperText?: ReactNode;

    items: T[];
    itemToString(item: null | T): string;
    itemToElement?(item: T): NonNullable<ReactNode>;
}
export function BoundDropdown<T>(props: BoundDropdownProps<T>) {
    const {
        id,

        // Labels
        labelText,
        placeholderText,
        helperText,
        decorator,

        // Value
        value,
        items,
        onChange,
        itemToString,
        itemToElement,

        // State
        disabled,
        error,
    } = props;

    return (
        <Dropdown<T>
            id={id}
            // This little shit seems to really need thrashing because otherwise
            // it remembers the last selected value even when it's on a different
            // parent entity and it should be nullified by the new one.
            key={`${id}-${value}`}
            autoAlign
            // className={css.dropdown}
            onChange={x => {
                const v = x.selectedItem;
                if (v != null) onChange?.(v);
            }}
            itemToString={itemToString}
            itemToElement={itemToElement}
            renderSelectedItem={itemToElement}
            items={items}
            selectedItem={value ?? undefined}
            titleText={labelText}
            label={placeholderText}
            decorator={decorator}
            helperText={helperText}
            invalid={!!error}
            invalidText={error}
            disabled={disabled}
        />
    );
}

export interface BoundRadioGroupProps<T extends string | number> extends iField<T> {
    id: string;
    labelText: string;
    items: Array<OptionItem<T>>;
    decorator?: ReactNode;
    helperText?: ReactNode;
}
export function BoundRadioGroup<T extends string | number>(props: BoundRadioGroupProps<T>) {
    const { id, labelText, helperText, decorator, value, items, onChange, disabled, error } = props;

    return (
        <RadioButtonGroup
            id={id}
            // This little shit seems to really need thrashing because otherwise
            // it remembers the last selected value even when it's on a different
            // parent entity and it should be nullified by the new one.
            key={`${id}-${value}`}
            name={id}
            value={value ?? undefined}
            legendText={labelText}
            children={items.map(x => (
                <RadioButton key={x.value} value={x.value} labelText={x.label} checked={value === x.value} />
            ))}
            onChange={v => onChange?.(v as T)}
            invalid={!!error}
            invalidText={error}
            helperText={helperText}
            decorator={decorator}
            disabled={disabled}
        />
    );
}

export function CheckYourScreenForPreview() {
    return (
        <div className={css.note}>
            <IconScreen size={16} />
            <FormattedMessage
                tagName="span"
                defaultMessage="<b>Note</b>: Check your device screen to see live preview"
                values={{ b: ch => <strong key="b">{ch}</strong> }}
            />
        </div>
    );
}
export function WidgetHasNoParametersToConfigure() {
    return (
        <div className={css.note}>
            <IconInfo size={16} />
            <FormattedMessage tagName="span" defaultMessage="This widget has no parameters to configure" />
        </div>
    );
}
