import css from './shared.scss';
import { FormattedMessage } from 'react-intl';

import * as pb from '@/proto';
import type { iField } from '@/lib/form';

import { useIntl } from 'react-intl';
import { ButtonSwitch, Checkbox } from '@/components';
import { RadioButtonGroup, RadioButton, ComboBox, Toggle } from '@carbon/react';
import { Screen as IconScreen } from '@carbon/react/icons';

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
            onChange={(_, { checked }) => onChange(checked)}
            invalid={!!error}
            invalidText={error}
        />
    );
}

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

    return (
        <ComboBox<OptionItem<T>>
            id={id}
            autoAlign
            className={css.comboBox}
            onChange={x => {
                const v = x.selectedItem?.value;
                if (v != null) onChange(v);
            }}
            itemToString={x => (x?.label ? String(x.label) : 'N/A')}
            items={items}
            selectedItem={value ? { label: value, value } : undefined}
            titleText={labelText}
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
            name={id}
            value={value ?? undefined}
            legendText={labelText}
            children={items.map(x => (
                <RadioButton key={x.value} value={x.value} labelText={x.label} checked={value === x.value} />
            ))}
            onChange={v => onChange(v as T)}
            invalid={!!error}
            invalidText={error}
            helperText={helperText}
            decorator={decorator}
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

export function CheckYourScreenForPreview() {
    return (
        <div className={css.note}>
            <IconScreen size={16} />
            <FormattedMessage
                tagName="span"
                defaultMessage="<b>Note</b>: Check your device screen to see live preview"
                values={{ b: ch => <strong children={ch} /> }}
            />
        </div>
    );
}
