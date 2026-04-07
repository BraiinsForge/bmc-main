import { useIntl } from 'react-intl';

import * as pb from '@/proto';
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

function parseJsonDefault(defaultValue: string): unknown {
    try {
        return JSON.parse(defaultValue);
    } catch {
        return defaultValue;
    }
}

export function encodeNumberParamValue(value: string | number | null): string {
    if (value === '' || value === null) return 'null';
    const numeric = typeof value === 'number' ? value : Number(value);
    if (Number.isNaN(numeric)) return 'null';
    return JSON.stringify(numeric);
}

export function encodeNumberEnumParamValue(value: string): string {
    return encodeNumberParamValue(value);
}

export function getNumberInputValue(value: string): number | '' {
    const parsedValue = parseJsonDefault(value);
    return typeof parsedValue === 'number' ? parsedValue : '';
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

    const parsedValue = parseJsonDefault(value);
    const enumEntries = Object.entries(definition.enumValues);

    switch (definition.paramType) {
        case pb.ManifestParamType.STRING: {
            if (enumEntries.length > 0) {
                const items: Array<OptionItem<string>> = enumEntries.map(([val, label]) => ({
                    value: val,
                    label,
                }));
                return (
                    <BoundComboBox<string>
                        id={id}
                        labelText={definition.name}
                        items={items}
                        value={typeof parsedValue === 'string' ? parsedValue : null}
                        onChange={v => onChange(definition.key, JSON.stringify(v))}
                    />
                );
            }

            return (
                <TextInput
                    id={id}
                    labelText={definition.name}
                    helperText={definition.description}
                    value={typeof parsedValue === 'string' ? parsedValue : ''}
                    onChange={e => onChange(definition.key, JSON.stringify(e.target.value))}
                />
            );
        }

        case pb.ManifestParamType.BOOLEAN:
            return (
                <BoundToggle
                    id={id}
                    labelText={definition.name}
                    value={parsedValue === true}
                    onChange={v => onChange(definition.key, JSON.stringify(v))}
                />
            );

        case pb.ManifestParamType.NUMBER: {
            if (enumEntries.length > 0) {
                const items: Array<OptionItem<string>> = enumEntries.map(([val, label]) => ({
                    value: val,
                    label,
                }));
                return (
                    <BoundComboBox<string>
                        id={id}
                        labelText={definition.name}
                        items={items}
                        value={String(parsedValue)}
                        onChange={v => {
                            onChange(definition.key, encodeNumberEnumParamValue(v));
                        }}
                    />
                );
            }

            return (
                <NumberInput
                    id={id}
                    label={definition.name}
                    helperText={definition.description}
                    value={getNumberInputValue(value)}
                    allowEmpty
                    min={definition.min}
                    max={definition.max}
                    onChange={(_e, { value }) => {
                        onChange(definition.key, encodeNumberParamValue(value));
                    }}
                />
            );
        }

        case pb.ManifestParamType.ARRAY:
            return (
                <TextInput
                    id={id}
                    labelText={definition.name}
                    helperText={definition.description ?? 'JSON array'}
                    value={value}
                    onChange={e => onChange(definition.key, e.target.value)}
                />
            );

        case pb.ManifestParamType.TIMEZONE: {
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
            return (
                <BoundComboBox<string>
                    id={id}
                    labelText={definition.name}
                    helperText={definition.description}
                    items={tzItems}
                    value={parsedValue === null ? '' : typeof parsedValue === 'string' ? parsedValue : null}
                    onChange={v => onChange(definition.key, v ? JSON.stringify(v) : 'null')}
                />
            );
        }

        default:
            return (
                <TextInput
                    id={id}
                    labelText={definition.name}
                    helperText={definition.description}
                    value={typeof parsedValue === 'string' ? parsedValue : String(parsedValue)}
                    onChange={e => onChange(definition.key, JSON.stringify(e.target.value))}
                />
            );
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
                    value={params[def.key] ?? def.defaultValue}
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
