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
    params: Record<string, pb.WidgetDataValue>;
    fieldErrors?: Record<string, string>;
    onParamChange(key: string, value: pb.WidgetDataValue | undefined): void;

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

function readString(value: pb.WidgetDataValue | undefined): string {
    return value?.kind.case === 'stringValue' ? value.kind.value : '';
}

function readNumber(value: pb.WidgetDataValue | undefined): number | '' {
    if (!value) return '';
    if (value.kind.case === 'integerValue' || value.kind.case === 'doubleValue') return value.kind.value;
    return '';
}

function readBoolean(value: pb.WidgetDataValue | undefined): boolean {
    return value?.kind.case === 'booleanValue' && value.kind.value === true;
}

function makeStringParamValue(v: string, isOptional: boolean): pb.WidgetDataValue {
    if (isOptional && v === '') return makeNullValue();
    return makeStringValue(v);
}

function makeNumberParamValue(v: string | number | null, type: 'integer' | 'double'): pb.WidgetDataValue {
    if (v === '' || v === null) return makeNullValue();
    const n = typeof v === 'number' ? v : Number(v);
    if (Number.isNaN(n)) return makeNullValue();
    return type === 'integer' ? makeIntegerValue(n) : makeDoubleValue(n);
}

function ParamField(props: {
    id: string;
    definition: pb.ManifestParamDefinition;
    value: pb.WidgetDataValue;
    error?: string;
    onChange(key: string, value: pb.WidgetDataValue | undefined): void;
    timezones: pb.Timezone[];
}) {
    const { id, definition, value, onChange, timezones, error } = props;
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
                        error={error}
                        items={items}
                        value={readString(value) || null}
                        onChange={v =>
                            onChange(
                                definition.key,
                                v !== null ? makeStringParamValue(v, definition.isOptional) : undefined,
                            )
                        }
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
                    value={readString(value)}
                    onChange={e =>
                        onChange(definition.key, makeStringParamValue(e.target.value, definition.isOptional))
                    }
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
                        error={error}
                        items={items}
                        value={String(readNumber(value))}
                        onChange={v => onChange(definition.key, makeNumberParamValue(v, 'integer'))}
                    />
                );
            }
            return (
                <NumberInput
                    id={id}
                    label={labelText}
                    helperText={definition.description}
                    invalid={!!error}
                    invalidText={error}
                    value={readNumber(value)}
                    allowEmpty
                    min={min}
                    max={max}
                    step={step ?? 1}
                    onChange={(_e, { value: v }) => onChange(definition.key, makeNumberParamValue(v, 'integer'))}
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
                        error={error}
                        items={items}
                        value={String(readNumber(value))}
                        onChange={v => onChange(definition.key, makeNumberParamValue(v, 'double'))}
                    />
                );
            }
            return (
                <NumberInput
                    id={id}
                    label={labelText}
                    helperText={definition.description}
                    invalid={!!error}
                    invalidText={error}
                    value={readNumber(value)}
                    allowEmpty
                    min={min}
                    max={max}
                    step={step ?? 0.01}
                    onChange={(_e, { value: v }) => onChange(definition.key, makeNumberParamValue(v, 'double'))}
                />
            );
        }

        case 'paramBoolean':
            return (
                <BoundToggle
                    id={id}
                    labelText={labelText}
                    error={error}
                    value={readBoolean(value)}
                    onChange={v => onChange(definition.key, makeBooleanValue(v))}
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
                    error={error}
                    items={tzItems}
                    value={tzValue || ''}
                    onChange={v => onChange(definition.key, v ? makeStringValue(v) : makeNullValue())}
                />
            );
        }

        default:
            return null;
    }
}

function kindDefaultValue(kind: pb.ManifestParamDefinition['kind']): pb.WidgetDataValue {
    switch (kind.case) {
        case 'paramString':
            return makeStringValue(kind.value.defaultValue ?? '');
        case 'paramTimezone':
            return kind.value.defaultValue !== undefined ? makeStringValue(kind.value.defaultValue) : makeNullValue();
        case 'paramInteger':
            return kind.value.defaultValue !== undefined ? makeIntegerValue(kind.value.defaultValue) : makeNullValue();
        case 'paramDouble':
            return kind.value.defaultValue !== undefined ? makeDoubleValue(kind.value.defaultValue) : makeNullValue();
        case 'paramBoolean':
            return makeBooleanValue(kind.value.defaultValue ?? false);
        default:
            return makeStringValue('');
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
        fieldErrors,
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
                    error={fieldErrors?.[def.key]}
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
