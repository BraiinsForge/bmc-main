import { useIntl } from 'react-intl';

import * as pb from '@/proto';
import { create } from '@/proto';
import { getID } from '../const';
import { Form } from '@/lib/form';

// Components
import { BoundToggle, BoundComboBox, type OptionItem, CheckYourScreenForPreview, WidgetSizeSelector } from '../shared';
import { ModalCustom, InlineNotification, Button } from '@/components';
import { TextInput, NumberInput } from '@carbon/react';

// Styles
import css from '../shared.scss';

const $ = getID('manifest-form').get;

export interface FormWidgetManifestProps {
    isOpen: boolean;
    onSave(): void;
    onCancel(): void;
    error: Maybe<string>;

    manifest: null | pb.WidgetManifest;
    params: Record<string, string>;
    onParamChange(key: string, value: string | undefined): void;

    /** Timezones available on the device, fetched once via `sys.getTimezoneList`. */
    timezones: pb.Timezone[];

    /** Current size selection for a combined-scene widget. Omit for fullscreen flows. */
    size?: pb.WidgetSize;
    /** Sizes the user may pick from (intersection of cell fit and manifest.supported_sizes). */
    sizeOptions?: Array<Exclude<pb.WidgetSize, pb.WidgetSize.UNSPECIFIED>>;
    onSizeChange?(size: pb.WidgetSize): void;
}

function stringFormatToInputType(format: pb.StringFormat | undefined): string {
    switch (format) {
        case pb.StringFormat.DATE:
            return 'date';
        case pb.StringFormat.TIME:
            return 'time';
        case pb.StringFormat.EMAIL:
            return 'email';
        case pb.StringFormat.URI:
            return 'url';
        default:
            return 'text';
    }
}

function readString(raw: string): string {
    try {
        const parsed: unknown = JSON.parse(raw);
        return typeof parsed === 'string' ? parsed : raw;
    } catch {
        return raw;
    }
}

function readNumber(raw: string): number | '' {
    try {
        const parsed: unknown = JSON.parse(raw);
        return typeof parsed === 'number' ? parsed : '';
    } catch {
        return '';
    }
}

function readBoolean(raw: string): boolean {
    try {
        const parsed: unknown = JSON.parse(raw);
        return parsed === true;
    } catch {
        return false;
    }
}

function encodeString(v: string): string {
    return JSON.stringify(v);
}

function encodeNumber(v: string | number | null): string {
    if (v === '' || v === null) return 'null';
    const n = typeof v === 'number' ? v : Number(v);
    if (Number.isNaN(n)) return 'null';
    return JSON.stringify(n);
}

function makeStringValue(s: string): pb.WidgetDataValue {
    return create(pb.WidgetDataValueSchema, { kind: { case: 'stringValue', value: s } });
}

function makeIntegerValue(n: number): pb.WidgetDataValue {
    return create(pb.WidgetDataValueSchema, { kind: { case: 'integerValue', value: n } });
}

function makeDoubleValue(n: number): pb.WidgetDataValue {
    return create(pb.WidgetDataValueSchema, { kind: { case: 'doubleValue', value: n } });
}

function makeBooleanValue(b: boolean): pb.WidgetDataValue {
    return create(pb.WidgetDataValueSchema, { kind: { case: 'booleanValue', value: b } });
}

function makeNullValue(): pb.WidgetDataValue {
    return create(pb.WidgetDataValueSchema, {
        kind: { case: 'nullValue', value: create(pb.WidgetDataValue_NullSchema) },
    });
}

export function widgetDataValueFromRaw(raw: string, kind: pb.ManifestParamDefinition['kind']): pb.WidgetDataValue {
    switch (kind.case) {
        case 'paramString':
        case 'paramTimezone':
            return makeStringValue(readString(raw));
        case 'paramInteger': {
            const n = readNumber(raw);
            return n === '' ? makeNullValue() : makeIntegerValue(n);
        }
        case 'paramDouble': {
            const n = readNumber(raw);
            return n === '' ? makeNullValue() : makeDoubleValue(n);
        }
        case 'paramBoolean':
            return makeBooleanValue(readBoolean(raw));
        default:
            return makeStringValue(readString(raw));
    }
}

function ParamField(props: {
    id: string;
    definition: pb.ManifestParamDefinition;
    value: string;
    onChange(key: string, value: string | undefined): void;
    timezones: pb.Timezone[];
}) {
    const { id, definition, value, onChange, timezones } = props;
    const { formatMessage } = useIntl();
    const required = !definition.isOptional;
    const labelText = required ? `${definition.name} *` : definition.name;

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
                        items={items}
                        value={readString(value) || null}
                        onChange={v => onChange(definition.key, v !== null ? encodeString(v) : undefined)}
                    />
                );
            }
            return (
                <TextInput
                    id={id}
                    labelText={labelText}
                    helperText={definition.description}
                    type={stringFormatToInputType(format)}
                    value={readString(value)}
                    onChange={e => onChange(definition.key, encodeString(e.target.value))}
                />
            );
        }

        case 'paramInteger': {
            const { min, max, step, enumValues } = definition.kind.value;
            if (enumValues.length > 0) {
                const items: Array<OptionItem<string>> = enumValues.map(opt => ({
                    value: String(opt.value),
                    label: opt.label,
                }));
                return (
                    <BoundComboBox<string>
                        id={id}
                        labelText={labelText}
                        items={items}
                        value={String(readNumber(value))}
                        onChange={v => onChange(definition.key, encodeNumber(v))}
                    />
                );
            }
            return (
                <NumberInput
                    id={id}
                    label={labelText}
                    helperText={definition.description}
                    value={readNumber(value)}
                    allowEmpty
                    min={min}
                    max={max}
                    step={step ?? 1}
                    onChange={(_e, { value: v }) => onChange(definition.key, encodeNumber(v))}
                />
            );
        }

        case 'paramDouble': {
            const { min, max, step, enumValues } = definition.kind.value;
            if (enumValues.length > 0) {
                const items: Array<OptionItem<string>> = enumValues.map(opt => ({
                    value: String(opt.value),
                    label: opt.label,
                }));
                return (
                    <BoundComboBox<string>
                        id={id}
                        labelText={labelText}
                        items={items}
                        value={String(readNumber(value))}
                        onChange={v => onChange(definition.key, encodeNumber(v))}
                    />
                );
            }
            return (
                <NumberInput
                    id={id}
                    label={labelText}
                    helperText={definition.description}
                    value={readNumber(value)}
                    allowEmpty
                    min={min}
                    max={max}
                    step={step ?? 0.01}
                    onChange={(_e, { value: v }) => onChange(definition.key, encodeNumber(v))}
                />
            );
        }

        case 'paramBoolean':
            return (
                <BoundToggle
                    id={id}
                    labelText={labelText}
                    value={readBoolean(value)}
                    onChange={v => onChange(definition.key, JSON.stringify(v))}
                />
            );

        case 'paramTimezone': {
            const tzItems: Array<OptionItem<string>> = [
                {
                    value: '',
                    label: formatMessage({ defaultMessage: 'System Timezone' }),
                },
                ...timezones.map(tz => ({
                    value: tz.id,
                    label: `${tz.offset} ${tz.label}`,
                })),
            ];
            const tzValue = readString(value);
            return (
                <BoundComboBox<string>
                    id={id}
                    labelText={labelText}
                    helperText={definition.description}
                    items={tzItems}
                    value={tzValue || ''}
                    onChange={v => onChange(definition.key, v ? encodeString(v) : 'null')}
                />
            );
        }

        default:
            return null;
    }
}

function kindDefaultValue(kind: pb.ManifestParamDefinition['kind']): string {
    switch (kind.case) {
        case 'paramString':
            return kind.value.defaultValue !== undefined ? encodeString(kind.value.defaultValue) : '""';
        case 'paramTimezone':
            return kind.value.defaultValue !== undefined ? encodeString(kind.value.defaultValue) : 'null';
        case 'paramInteger':
        case 'paramDouble':
            return kind.value.defaultValue !== undefined ? JSON.stringify(kind.value.defaultValue) : 'null';
        case 'paramBoolean':
            return JSON.stringify(kind.value.defaultValue ?? false);
        default:
            return '""';
    }
}

export function FormWidgetManifest(props: FormWidgetManifestProps) {
    const {
        isOpen,
        onSave,
        onCancel,
        error,
        manifest,
        params,
        onParamChange,
        timezones,
        size,
        sizeOptions,
        onSizeChange,
    } = props;
    const { formatMessage } = useIntl();

    if (!manifest) return null;

    const showSizeSelector = !!sizeOptions && sizeOptions.length > 0 && !!onSizeChange && size != null;

    const form = (
        <Form className={css.form}>
            {showSizeSelector ? (
                <WidgetSizeSelector
                    field={{
                        value: size,
                        options: sizeOptions,
                        onChange: onSizeChange,
                    }}
                />
            ) : null}

            {manifest.params.map(def => (
                <ParamField
                    key={def.key}
                    id={$(`param-${def.key}`)}
                    definition={def}
                    value={params[def.key] ?? kindDefaultValue(def.kind)}
                    onChange={onParamChange}
                    timezones={timezones}
                />
            ))}

            <CheckYourScreenForPreview />

            {error ? (
                <InlineNotification
                    kind="error"
                    theme="inverse"
                    stretch
                    hideCloseButton
                    title={formatMessage({ defaultMessage: 'Error' })}
                    children={error}
                />
            ) : null}
        </Form>
    );

    return (
        <ModalCustom
            id={$('dialog')}
            className={css.modal}
            selectorPrimaryFocus="form input,button"
            size="sm"
            open={isOpen}
            title={manifest.name}
            label={formatMessage({ defaultMessage: 'Configure Widget' })}
            onClose={onCancel}
            children={form}
            footer={
                <Button
                    id={$('done')}
                    kind="primary"
                    children={formatMessage({ defaultMessage: 'Done' })}
                    onClick={onSave}
                />
            }
        />
    );
}
