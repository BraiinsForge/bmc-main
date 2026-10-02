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
import { type IntlShape, useIntl } from 'react-intl';
import {
    ComboBox,
    DatePicker,
    DatePickerInput,
    NumberInput,
    PasswordInput,
    RadioButton,
    RadioButtonGroup,
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
import cn from 'clsx';

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
    placeholder?: string;
}
export function BoundComboBox<T extends string | number>(props: BoundComboBoxProps<T>) {
    const { id, labelText, hideLabel, helperText, decorator, placeholder, value, items, onChange, disabled, error } =
        props;
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
            placeholder={placeholder}
            invalid={!!error}
            invalidText={error}
            disabled={disabled}
        />
    );
}

export interface BoundRadioGroupProps<T extends string | number> extends iField<T> {
    id: string;
    labelText: string;
    hideLabel?: boolean;
    items: Array<OptionItem<T>>;
    decorator?: ReactNode;
    helperText?: ReactNode;
}
export function BoundRadioGroup<T extends string | number>(props: BoundRadioGroupProps<T>) {
    const { id, labelText, hideLabel, helperText, decorator, value, items, onChange, disabled, error } = props;

    return (
        <RadioButtonGroup
            id={id}
            name={id}
            // '' keeps the group controlled when nothing is selected:
            // an undefined valueSelected goes uncontrolled, and a clicked radio
            // would then outlive the switch to another widget.
            valueSelected={value ?? ''}
            legendText={hideLabel ? <span className="cds--visually-hidden" children={labelText} /> : labelText}
            children={items.map(x => <RadioButton key={x.value} value={x.value} labelText={x.label} />)}
            onChange={v => onChange?.(v as T)}
            invalid={!!error}
            invalidText={error}
            helperText={helperText}
            decorator={decorator}
            disabled={disabled}
        />
    );
}

interface EnumFieldProps {
    id: string;
    labelText: string;
    hideLabel?: boolean;
    control: pb.EnumControl | undefined;
    items: Array<OptionItem<string>>;
    placeholder?: string;
    error?: string | ReactElement;
    value: string | null;
    onChange(value: string): void;
}

/** A choice among `enum_values`, drawn as the manifest's `enum_control` asks. */
function EnumField(props: EnumFieldProps) {
    const { control, placeholder, ...field } = props;
    if (control === pb.EnumControl.RADIO) return <BoundRadioGroup<string> {...field} />;
    return <BoundComboBox<string> {...field} placeholder={placeholder} />;
}

export interface BoundToggleProps extends iField<boolean> {
    id: string;
    labelText: string;
    hideLabel?: boolean;
}
export function BoundToggle(props: BoundToggleProps) {
    const { id, labelText, hideLabel, value, onChange, disabled, error } = props;
    const { formatMessage } = useIntl();
    const hiddenLabelId = `${id}-label`;
    const errorId = `${id}-error`;

    const toggle = (
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
            aria-invalid={error ? true : undefined}
            aria-errormessage={error ? errorId : undefined}
        />
    );

    return (
        <Fragment>
            {/* Not Carbon's `hideLabel`: it shows `labelText` in place of the On/Off side label. */}
            {hideLabel ? <span id={hiddenLabelId} className="cds--visually-hidden" children={labelText} /> : null}
            {hideLabel ? <div className={css.toggleBox} children={toggle} /> : toggle}
            {error ? <div id={errorId} className={css.toggleError} children={error} /> : null}
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
    /** An error of the row this field belongs to, shown once under the row: here it only marks the field. */
    rowError?: string;
    onChange(value: ScalarValue): void;
    timezones: pb.Timezone[];
}

function ScalarField(props: ScalarFieldProps) {
    const { id, kind, labelText, hideLabel, helperText, isOptional, value, onChange, timezones } = props;
    // A row error stays this field's message, hidden, so assistive tech still reads it on focus.
    const error =
        props.error ??
        (props.rowError ? <span className="cds--visually-hidden" children={props.rowError} /> : undefined);
    const { formatMessage } = useIntl();

    switch (kind.case) {
        case 'paramString': {
            const { enumValues, enumControl, format, placeholder } = kind.value;
            if (enumValues.length > 0) {
                const items: Array<OptionItem<string>> = enumValues.map(opt => ({
                    value: opt.value,
                    label: opt.label,
                }));
                return (
                    <EnumField
                        id={id}
                        labelText={labelText}
                        hideLabel={hideLabel}
                        control={enumControl}
                        placeholder={placeholder}
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
                            placeholder={placeholder ?? 'yyyy-mm-dd'}
                            pattern="\d{4}-\d{2}-\d{2}"
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
                        placeholder={placeholder}
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
                    placeholder={placeholder}
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
                    <EnumField
                        id={id}
                        labelText={labelText}
                        hideLabel={hideLabel}
                        control={inner.enumControl}
                        placeholder={inner.placeholder}
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
                    placeholder={inner.placeholder}
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
                    placeholder={kind.value.placeholder}
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

function unitOf(kind: ScalarKind | pb.ArrayItemKind['kind'] | undefined): string | undefined {
    return kind?.case === 'paramInteger' || kind?.case === 'paramDouble' ? kind.value.unit : undefined;
}

/** A field's name as every label shows it: with its unit, and flagged when optional. */
function displayName(
    name: string,
    isOptional: boolean,
    unit: string | undefined,
    formatMessage: IntlShape['formatMessage'],
): string {
    if (unit && isOptional) return formatMessage({ defaultMessage: '{name} ({unit}, optional)' }, { name, unit });
    if (unit) return formatMessage({ defaultMessage: '{name} ({unit})' }, { name, unit });
    if (isOptional) return formatMessage({ defaultMessage: '{name} (optional)' }, { name });
    return name;
}

function fieldName(field: pb.ObjectFieldDefinition, formatMessage: IntlShape['formatMessage']): string {
    return displayName(field.name, field.isOptional, unitOf(field.kind), formatMessage);
}

function ColumnLabel({ field }: { field: pb.ObjectFieldDefinition }) {
    const { formatMessage } = useIntl();
    const name = fieldName(field, formatMessage);
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
                {object.fields.map(field => {
                    const fieldError = ownValue(error?.fields, field.key);
                    const rowError = !fieldError && marksField(error, field.key) ? error?.error : undefined;
                    return (
                        <div key={field.key} className={cn(css.objectField, rowError && css.markedField)}>
                            <ScalarField
                                id={`${id}-${field.key}`}
                                kind={field.kind}
                                labelText={formatMessage(
                                    { defaultMessage: '{row}, {field}' },
                                    { row: labelText, field: fieldName(field, formatMessage) },
                                )}
                                hideLabel
                                isOptional={field.isOptional}
                                value={ownValue(value, field.key) ?? null}
                                error={fieldError}
                                rowError={rowError}
                                onChange={next => onChange({ ...value, [field.key]: next })}
                                timezones={timezones}
                            />
                        </div>
                    );
                })}
            </div>
        </CarbonFormField>
    );
}

function marksField(error: RowError | undefined, key: string): boolean {
    return !!error?.error && (error.markedFields?.includes(key) ?? true);
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
    const rowLabelOf = (index: number) =>
        formatMessage({ defaultMessage: '{name}, item {n}' }, { name: labelText, n: index + 1 });

    const listRef = useRef<HTMLDivElement>(null);
    const focusAfterRender = useRef<(() => HTMLElement | null | undefined) | null>(null);

    // No deps: the target only renders once
    // the parent passes the new value back down.
    useEffect(() => {
        const target = focusAfterRender.current?.();
        if (!target) return;
        focusAfterRender.current = null;
        target.focus();
    });

    const firstFieldOf = (rowId: ListItem['id']) =>
        listRef.current
            ?.querySelector(`[data-list-row="${rowId}"]`)
            ?.querySelector<HTMLElement>('input, button, textarea, select');

    function add(): void {
        const row = listItem(defaultItemValue(itemKind));
        focusAfterRender.current = () => firstFieldOf(row.id);
        onChange([...value, row]);
    }

    // A keyboard user's focus leaves the button before
    // it unmounts, which would drop it to the page;
    // a mouse click is blurred by `Button` anyway.
    const remove = (row: ListItem) => {
        const index = value.indexOf(row);
        const rest = value.filter(x => x.id !== row.id);
        const neighbour = rest[index] ?? rest[index - 1];
        if (neighbour) {
            // At `min_items` the remaining remove buttons
            // turn disabled, which would drop focus again.
            const target =
                rest.length > array.minItems
                    ? document.getElementById(`${id}-${neighbour.id}-remove`)
                    : firstFieldOf(neighbour.id);

            target?.focus();
        } else {
            // At `max_items: 1`, Add stays disabled until the emptied list renders.
            focusAfterRender.current = () => {
                const button = document.getElementById(`${id}-add`);
                return button instanceof HTMLButtonElement && !button.disabled ? button : null;
            };
        }
        onChange(rest);
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
                getItemLabel={item => rowLabelOf(value.indexOf(item))}
                renderItem={({ index, item, rootProps, dragHandleProps }) => {
                    // The drag overlay renders a second copy of the row, without `rootProps`.
                    const rowId = rootProps ? `${id}-${item.id}` : `${id}-${item.id}-dragged`;
                    const rowLabel = rowLabelOf(index);
                    return (
                        <div {...rootProps} className={css.row}>
                            <div
                                {...dragHandleProps}
                                className={css.dragHandle}
                                children={
                                    <IconDraggable
                                        aria-label={formatMessage({ defaultMessage: 'Move {row}' }, { row: rowLabel })}
                                    />
                                }
                            />
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
                                    tooltipPosition="left"
                                    title={formatMessage({ defaultMessage: 'Remove {row}' }, { row: rowLabel })}
                                    disabled={!canRemove}
                                    onClick={() => remove(item)}
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
                    aria-label={formatMessage({ defaultMessage: 'Add to {list}' }, { list: labelText })}
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
    const { kind } = definition;
    // Carbon convention: required is the norm (unmarked); flag only the optional fields.
    // A list's unit is its items'.
    const unit = unitOf(kind.case === 'paramArray' ? kind.value.items?.kind : kind);
    const labelText = displayName(definition.name, definition.isOptional, unit, formatMessage);

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
