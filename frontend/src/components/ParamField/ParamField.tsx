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

import { Fragment, useEffect, useMemo, useRef, type ReactNode } from 'react';
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
import {
    Add as IconAdd,
    Draggable as IconDraggable,
    Information as IconInfo,
    SubtractAlt as IconSubtract,
} from '@carbon/react/icons';
import * as pb from '@/proto';
import type { iField } from '@/lib/form';
import { useIsTouchDevice } from '@/lib/react';
import { assertUnreachable, ownValue } from '@/lib/ts';
import { Button } from '@/components/Button';
import { CarbonFormField } from '@/components/CarbonFormField';
import { Sortable } from '@/components/Sortable';
import { Tooltip } from '@/components/Tooltip';
import {
    defaultItemValue,
    listItem,
    type FieldValue,
    type ListItem,
    type ObjectValue,
    type RowError,
    type ScalarKind,
    type ScalarValue,
} from './value';

// Styles
import css from './ParamField.scss';

export interface OptionItem<T extends string | number> {
    value: T;
    label: number | string;
}

export interface BoundComboBoxProps<T extends string | number> extends iField<T> {
    id: string;
    labelText: string;
    hideLabel?: boolean;
    items: Array<OptionItem<T>>;
    decorator?: ReactNode;
    helperText?: ReactNode;
}
export function BoundComboBox<T extends string | number>(props: BoundComboBoxProps<T>) {
    const { id, labelText, hideLabel, helperText, decorator, value, items, onChange, disabled, error } = props;
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
                hideLabel={hideLabel}
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
            titleText={hideLabel ? undefined : labelText}
            aria-label={hideLabel ? labelText : undefined}
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
    hideLabel?: boolean;
}
export function BoundToggle(props: BoundToggleProps) {
    const { id, labelText, hideLabel, value, onChange, disabled } = props;
    const { formatMessage } = useIntl();
    const hiddenLabelId = `${id}-label`;

    return (
        <Fragment>
            {/* Not Carbon's `hideLabel`: it shows `labelText` in place of the On/Off side label. */}
            {hideLabel ? <span id={hiddenLabelId} className="cds--visually-hidden" children={labelText} /> : null}
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
                labelText={hideLabel ? undefined : labelText}
                aria-labelledby={hideLabel ? hiddenLabelId : undefined}
            />
        </Fragment>
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

function asString(v: ScalarValue): string {
    return typeof v === 'string' ? v : '';
}
function asBoolean(v: ScalarValue): boolean {
    return v === true;
}
function asScalar(v: FieldValue): ScalarValue {
    return Array.isArray(v) ? null : v;
}
function asList(v: FieldValue): ListItem[] {
    return Array.isArray(v) ? v : [];
}

interface ScalarFieldProps {
    id: string;
    kind: ScalarKind;
    labelText: string;
    hideLabel?: boolean;
    helperText?: string;
    isOptional: boolean;
    value: ScalarValue;
    error?: string;
    onChange(value: ScalarValue): void;
    timezones: pb.Timezone[];
}

function ScalarField(props: ScalarFieldProps) {
    const { id, kind, labelText, hideLabel, helperText, isOptional, value, error, onChange, timezones } = props;
    const { formatMessage } = useIntl();

    switch (kind.case) {
        case 'paramString': {
            const { enumValues, format } = kind.value;
            if (enumValues.length > 0) {
                const items: Array<OptionItem<string>> = enumValues.map(opt => ({
                    value: opt.value,
                    label: opt.label,
                }));
                return (
                    <BoundComboBox<string>
                        id={id}
                        labelText={labelText}
                        hideLabel={hideLabel}
                        error={error}
                        items={items}
                        value={asString(value) || null}
                        onChange={onChange}
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
                        onChange={(_dates, dateStr) => onChange(dateStr)}
                    >
                        <DatePickerInput
                            id={id}
                            labelText={labelText}
                            hideLabel={hideLabel}
                            helperText={helperText}
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
                        hideLabel={hideLabel}
                        helperText={helperText}
                        invalid={!!error}
                        invalidText={error}
                        tooltipPosition="left"
                        value={asString(value)}
                        onChange={e => onChange(e.target.value)}
                    />
                );
            }
            return (
                <TextInput
                    id={id}
                    labelText={labelText}
                    hideLabel={hideLabel}
                    helperText={helperText}
                    invalid={!!error}
                    invalidText={error}
                    type={stringFormatToInputType(format)}
                    value={asString(value)}
                    onChange={e => onChange(e.target.value)}
                />
            );
        }

        case 'paramInteger':
        case 'paramDouble': {
            const isInt = kind.case === 'paramInteger';
            const inner = kind.value;
            if (inner.enumValues.length > 0) {
                const items: Array<OptionItem<string>> = inner.enumValues.map(opt => ({
                    value: String(opt.value),
                    label: opt.label,
                }));
                return (
                    <BoundComboBox<string>
                        id={id}
                        labelText={labelText}
                        hideLabel={hideLabel}
                        error={error}
                        items={items}
                        value={asString(value)}
                        onChange={onChange}
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
                    onChange('NaN');
                } else {
                    onChange(String(state.value));
                }
            };
            return (
                <NumberInput
                    id={id}
                    label={labelText}
                    hideLabel={hideLabel}
                    helperText={helperText}
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
                    hideLabel={hideLabel}
                    error={error}
                    value={asBoolean(value)}
                    onChange={onChange}
                />
            );

        case 'paramTimezone': {
            const tzItems: Array<OptionItem<string>> = [
                ...(isOptional ? [{ value: '', label: formatMessage({ defaultMessage: 'System Timezone' }) }] : []),
                ...timezones.map(tz => ({ value: tz.id, label: `${tz.offset} ${tz.label}` })),
            ];
            return (
                <BoundComboBox<string>
                    id={id}
                    labelText={labelText}
                    hideLabel={hideLabel}
                    helperText={helperText}
                    error={error}
                    items={tzItems}
                    value={value === null ? '' : asString(value)}
                    onChange={onChange}
                />
            );
        }

        case undefined:
            return null;

        default:
            return assertUnreachable(kind, 'scalar param kind');
    }
}

function asRowScalar(v: ListItem['value']): ScalarValue {
    return typeof v === 'object' && v !== null ? null : v;
}
function asRowObject(v: ListItem['value']): ObjectValue {
    return typeof v === 'object' && v !== null ? v : {};
}

function ColumnLabel({ field }: { field: pb.ObjectFieldDefinition }) {
    const { formatMessage } = useIntl();
    const name = field.isOptional
        ? formatMessage({ defaultMessage: '{name} (optional)' }, { name: field.name })
        : field.name;
    if (!field.description) return <span className={css.columnLabel} children={name} />;
    return (
        <Tooltip
            placement="top"
            content={field.description}
            render={ref => (
                <span ref={ref} className={css.columnLabel}>
                    <span children={name} />
                    <IconInfo size={14} />
                </span>
            )}
        />
    );
}

function ObjectHeader({ object }: { object: pb.ParamObject }) {
    return (
        <div className={css.header} aria-hidden>
            <div className={css.handleSpacer} />
            <div className={css.headerFields}>
                {object.fields.map(field => (
                    <div key={field.key} className={css.objectField} children={<ColumnLabel field={field} />} />
                ))}
            </div>
            <div className={css.actionSpacer} />
        </div>
    );
}

interface ObjectRowProps {
    id: string;
    object: pb.ParamObject;
    labelText: string;
    value: ObjectValue;
    error?: RowError;
    onChange(value: ObjectValue): void;
    timezones: pb.Timezone[];
}

function ObjectRow({ id, object, labelText, value, error, onChange, timezones }: ObjectRowProps) {
    const { formatMessage } = useIntl();
    return (
        <CarbonFormField error={error?.error}>
            <div className={css.objectFields}>
                {object.fields.map(field => (
                    <div key={field.key} className={css.objectField}>
                        <ScalarField
                            id={`${id}-${field.key}`}
                            kind={field.kind}
                            labelText={formatMessage(
                                { defaultMessage: '{row}, {field}' },
                                { row: labelText, field: field.name },
                            )}
                            hideLabel
                            isOptional={field.isOptional}
                            value={ownValue(value, field.key) ?? null}
                            error={ownValue(error?.fields, field.key)}
                            onChange={next => onChange({ ...value, [field.key]: next })}
                            timezones={timezones}
                        />
                    </div>
                ))}
            </div>
        </CarbonFormField>
    );
}

interface ArrayFieldProps {
    id: string;
    array: pb.ParamArray;
    labelText: string;
    helperText?: string;
    value: ListItem[];
    error?: string;
    itemErrors?: Array<RowError | undefined>;
    onChange(value: ListItem[]): void;
    timezones: pb.Timezone[];
}

function ArrayField(props: ArrayFieldProps) {
    const { id, array, labelText, helperText, value, error, itemErrors, onChange, timezones } = props;
    const { formatMessage } = useIntl();
    const itemKind: pb.ArrayItemKind['kind'] = array.items?.kind ?? { case: undefined };
    const canRemove = value.length > array.minItems;
    const canAdd = value.length < array.maxItems;

    const setItem = (row: ListItem, next: ListItem['value']) =>
        onChange(value.map(x => (x.id === row.id ? { ...x, value: next } : x)));

    const listRef = useRef<HTMLDivElement>(null);
    const addedRow = useRef<ListItem['id'] | null>(null);
    // No deps: the added row only renders once the parent passes the new value back down.
    useEffect(() => {
        if (addedRow.current === null) return;
        const field = listRef.current?.querySelector(`[data-list-row="${addedRow.current}"]`);
        if (!field) return;
        addedRow.current = null;
        field.querySelector<HTMLElement>('input, button, textarea, select')?.focus();
    });
    const add = () => {
        const row = listItem(defaultItemValue(itemKind));
        addedRow.current = row.id;
        onChange([...value, row]);
    };

    return (
        <CarbonFormField labelText={labelText}>
            {error ? (
                <div role="alert" className={css.listError} children={error} />
            ) : helperText ? (
                <div className={css.listHelper} children={helperText} />
            ) : null}
            {itemKind.case === 'paramObject' ? <ObjectHeader object={itemKind.value} /> : null}
            <Sortable<ListItem>
                wrapperRef={listRef}
                className={css.list}
                items={value}
                onChange={onChange}
                renderItem={({ index, item, rootProps, dragHandleProps }) => {
                    // The drag overlay renders a second copy of the row, without `rootProps`.
                    const rowId = rootProps ? `${id}-${item.id}` : `${id}-${item.id}-dragged`;
                    const rowLabel = formatMessage(
                        { defaultMessage: '{name}, item {n}' },
                        { name: labelText, n: index + 1 },
                    );
                    return (
                        <div {...rootProps} className={css.row}>
                            <div {...dragHandleProps} className={css.dragHandle} children={<IconDraggable />} />
                            <div className={css.rowField} data-list-row={item.id}>
                                {itemKind.case === 'paramObject' ? (
                                    <ObjectRow
                                        id={rowId}
                                        object={itemKind.value}
                                        labelText={rowLabel}
                                        value={asRowObject(item.value)}
                                        error={itemErrors?.[index]}
                                        onChange={next => setItem(item, next)}
                                        timezones={timezones}
                                    />
                                ) : (
                                    <ScalarField
                                        id={rowId}
                                        kind={itemKind}
                                        labelText={rowLabel}
                                        hideLabel
                                        isOptional={false}
                                        value={asRowScalar(item.value)}
                                        error={itemErrors?.[index]?.error}
                                        onChange={next => setItem(item, next)}
                                        timezones={timezones}
                                    />
                                )}
                            </div>
                            <div className={css.rowAction}>
                                <Button
                                    id={`${rowId}-remove`}
                                    kind="danger--ghost"
                                    size="sm"
                                    icon={IconSubtract}
                                    title={formatMessage({ defaultMessage: 'Remove' })}
                                    disabled={!canRemove}
                                    onClick={() => onChange(value.filter(x => x.id !== item.id))}
                                />
                            </div>
                        </div>
                    );
                }}
            />
            <div className={css.footer}>
                <Button
                    id={`${id}-add`}
                    kind="tertiary"
                    size="sm"
                    icon={IconAdd}
                    disabled={!canAdd}
                    onClick={add}
                    children={formatMessage({ defaultMessage: 'Add' })}
                />
            </div>
        </CarbonFormField>
    );
}

export function ParamField(props: {
    id: string;
    definition: pb.ManifestParamDefinition;
    value: FieldValue;
    error?: string;
    itemErrors?: Array<RowError | undefined>;
    onChange(key: string, value: FieldValue): void;
    timezones: pb.Timezone[];
}) {
    const { id, definition, value, error, itemErrors, onChange, timezones } = props;
    const { formatMessage } = useIntl();
    // Carbon convention: required is the norm (unmarked); flag only the optional fields.
    const labelText = definition.isOptional
        ? formatMessage({ defaultMessage: '{name} (optional)' }, { name: definition.name })
        : definition.name;
    const { kind } = definition;

    if (kind.case === 'paramArray') {
        return (
            <ArrayField
                id={id}
                array={kind.value}
                labelText={labelText}
                helperText={definition.description}
                value={asList(value)}
                error={error}
                itemErrors={itemErrors}
                onChange={v => onChange(definition.key, v)}
                timezones={timezones}
            />
        );
    }
    return (
        <ScalarField
            id={id}
            kind={kind}
            labelText={labelText}
            helperText={definition.description}
            isOptional={definition.isOptional}
            value={asScalar(value)}
            error={error}
            onChange={v => onChange(definition.key, v)}
            timezones={timezones}
        />
    );
}
