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

// The shared field renderer — one control per field kind — plus the bound form controls it needs.

import { useMemo, type ReactNode } from 'react';
import { useIntl } from 'react-intl';
import {
    ComboBox,
    DatePicker,
    DatePickerInput,
    NumberInput,
    PasswordInput,
    Select,
    SelectItem,
    TextInput,
    Toggle,
} from '@carbon/react';
import * as pb from '@/proto';
import type { iField } from '@/lib/form';
import { useIsTouchDevice } from '@/lib/react';
import { assertUnreachable } from '@/lib/ts';

// Styles
import css from './ParamField.scss';

// Structurally identical to the widget form's `FormifiedValue`, so either is accepted.
export type FieldValue = string | boolean | null;

export interface OptionItem<T extends string | number> {
    value: T;
    label: number | string;
}

export interface BoundComboBoxProps<T extends string | number> extends iField<T> {
    id: string;
    labelText: string;
    items: Array<OptionItem<T>>;
    decorator?: ReactNode;
    helperText?: ReactNode;
}
export function BoundComboBox<T extends string | number>(props: BoundComboBoxProps<T>) {
    const { id, labelText, helperText, decorator, value, items, onChange, disabled, error } = props;
    const isTouchDevice = useIsTouchDevice();

    const selectedItemStruct = useMemo<undefined | OptionItem<T>>(() => {
        const x = items.find(x => x.value === value);
        return x ? { value: x.value, label: x.label } : undefined;
    }, [value, items]);

    // On touch devices, use native select for better UX
    // (uses OS picker on mobile - iOS wheel, Android spinner)
    if (isTouchDevice) {
        return (
            <Select
                id={id}
                labelText={labelText}
                helperText={helperText}
                decorator={decorator}
                value={value ?? undefined}
                onChange={e => onChange?.(e.target.value as T)}
                invalid={!!error}
                invalidText={error}
                disabled={disabled}
                children={items.map(item => (
                    <SelectItem key={item.value} value={item.value} text={String(item.label)} />
                ))}
            />
        );
    }

    return (
        <ComboBox<OptionItem<T>>
            id={id}
            // This little shit seems to really need thrashing because otherwise
            // it remembers the last selected value even when it's on a different
            // parent entity and it should be nullified by the new one.
            key={`${id}-${value}`}
            autoAlign
            className={css.comboBox}
            onChange={x => {
                const v = x.selectedItem?.value;
                if (v != null) onChange?.(v);
            }}
            itemToString={x => (x?.label ? String(x.label) : '')}
            items={items}
            selectedItem={selectedItemStruct}
            titleText={labelText}
            decorator={decorator}
            helperText={helperText}
            invalid={!!error}
            invalidText={error}
            disabled={disabled}
        />
    );
}

export interface BoundToggleProps extends iField<boolean> {
    id: string;
    labelText: string;
}
export function BoundToggle(props: BoundToggleProps) {
    const { id, labelText, value, onChange, disabled } = props;
    const { formatMessage } = useIntl();

    return (
        <Toggle
            id={id}
            // This little shit seems to really need thrashing because otherwise
            // it remembers the last selected value even when it's on a different
            // parent entity and it should be nullified by the new one.
            key={`${id}-${value}`}
            size="md"
            toggled={!!value}
            onToggle={onChange}
            disabled={disabled}
            labelA={formatMessage({ defaultMessage: 'Off' })}
            labelB={formatMessage({ defaultMessage: 'On' })}
            labelText={labelText}
        />
    );
}

function stringFormatToInputType(format: pb.StringFormat | undefined): string {
    switch (format) {
        // DATE is handled by `DatePicker` before this is reached.
        case pb.StringFormat.TIME:
            return 'time';
        case pb.StringFormat.EMAIL:
            return 'email';
        case pb.StringFormat.URI:
            return 'url';
        // PASSWORD is handled by `PasswordInput` before this is reached.
        default:
            return 'text';
    }
}

function asString(v: FieldValue): string {
    return typeof v === 'string' ? v : '';
}
function asBoolean(v: FieldValue): boolean {
    return v === true;
}

export function ParamField(props: {
    id: string;
    definition: pb.ManifestParamDefinition;
    value: FieldValue;
    error?: string;
    onChange(key: string, value: FieldValue): void;
    timezones: pb.Timezone[];
}) {
    const { id, definition, value, onChange, timezones, error } = props;
    const { formatMessage } = useIntl();
    // Carbon convention: required is the norm (unmarked); flag only the optional fields.
    const labelText = definition.isOptional
        ? formatMessage({ defaultMessage: '{name} (optional)' }, { name: definition.name })
        : definition.name;

    switch (definition.kind.case) {
        case 'paramString': {
            const { enumValues, format } = definition.kind.value;
            if (enumValues.length > 0) {
                const items: Array<OptionItem<string>> = enumValues.map(opt => ({
                    value: opt.value,
                    label: opt.label,
                }));
                return (
                    <BoundComboBox<string>
                        id={id}
                        labelText={labelText}
                        error={error}
                        items={items}
                        value={asString(value) || null}
                        onChange={v => onChange(definition.key, v)}
                    />
                );
            }
            if (format === pb.StringFormat.DATE) {
                return (
                    // Carbon draws its own calendar and indicator, so the control
                    // follows the theme — native date chrome ignores it.
                    //
                    // flatpickr's second argument is the value already formatted
                    // to `dateFormat`, which keeps the wire value ISO.
                    <DatePicker
                        className={css.datePicker}
                        datePickerType="single"
                        dateFormat="Y-m-d"
                        value={asString(value)}
                        onChange={(_dates, dateStr) => onChange(definition.key, dateStr)}
                    >
                        <DatePickerInput
                            id={id}
                            labelText={labelText}
                            helperText={definition.description}
                            invalid={!!error}
                            invalidText={error}
                            placeholder="yyyy-mm-dd"
                        />
                    </DatePicker>
                );
            }
            if (format === pb.StringFormat.PASSWORD) {
                return (
                    <PasswordInput
                        id={id}
                        labelText={labelText}
                        helperText={definition.description}
                        invalid={!!error}
                        invalidText={error}
                        tooltipPosition="left"
                        value={asString(value)}
                        onChange={e => onChange(definition.key, e.target.value)}
                    />
                );
            }
            return (
                <TextInput
                    id={id}
                    labelText={labelText}
                    helperText={definition.description}
                    invalid={!!error}
                    invalidText={error}
                    type={stringFormatToInputType(format)}
                    value={asString(value)}
                    onChange={e => onChange(definition.key, e.target.value)}
                />
            );
        }

        case 'paramInteger':
        case 'paramDouble': {
            const isInt = definition.kind.case === 'paramInteger';
            const inner = definition.kind.value;
            if (inner.enumValues.length > 0) {
                const items: Array<OptionItem<string>> = inner.enumValues.map(opt => ({
                    value: String(opt.value),
                    label: opt.label,
                }));
                return (
                    <BoundComboBox<string>
                        id={id}
                        labelText={labelText}
                        error={error}
                        items={items}
                        value={asString(value)}
                        onChange={v => onChange(definition.key, v)}
                    />
                );
            }
            const numericValue = (() => {
                if (typeof value !== 'string' || value === '') return '';
                const n = Number(value);
                return Number.isFinite(n) ? n : '';
            })();
            const handleNumberChange = (e: { target: EventTarget | null }, state: { value: number | string }) => {
                const tgt = e.target;
                // badInput ⇒ browser sends empty string, only validity.badInput distinguishes "empty" from
                // "non-numeric", so we emit 'NaN' as a parse-shape signal.
                if (tgt instanceof HTMLInputElement && tgt.validity.badInput) {
                    onChange(definition.key, 'NaN');
                } else {
                    onChange(definition.key, String(state.value));
                }
            };
            return (
                <NumberInput
                    id={id}
                    label={labelText}
                    helperText={definition.description}
                    invalid={!!error}
                    invalidText={error}
                    type="number"
                    allowEmpty
                    value={numericValue}
                    min={inner.min}
                    max={inner.max}
                    step={inner.step ?? (isInt ? 1 : 0.01)}
                    onChange={handleNumberChange}
                />
            );
        }

        case 'paramBoolean':
            return (
                <BoundToggle
                    id={id}
                    labelText={labelText}
                    error={error}
                    value={asBoolean(value)}
                    onChange={v => onChange(definition.key, v)}
                />
            );

        case 'paramTimezone': {
            const tzItems: Array<OptionItem<string>> = [
                ...(definition.isOptional
                    ? [{ value: '', label: formatMessage({ defaultMessage: 'System Timezone' }) }]
                    : []),
                ...timezones.map(tz => ({ value: tz.id, label: `${tz.offset} ${tz.label}` })),
            ];
            return (
                <BoundComboBox<string>
                    id={id}
                    labelText={labelText}
                    helperText={definition.description}
                    error={error}
                    items={tzItems}
                    value={value === null ? '' : asString(value)}
                    onChange={v => onChange(definition.key, v)}
                />
            );
        }

        case undefined:
            return null;

        default:
            return assertUnreachable(definition.kind, 'manifest param kind');
    }
}
