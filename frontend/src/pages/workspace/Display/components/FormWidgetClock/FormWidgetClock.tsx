import { Component } from 'react';
import { FormattedMessage, useIntl, type IntlShape } from 'react-intl';

import * as pb from '@/proto';
import { Form, type iField, getID } from '@/lib/form';

// Components
import { ModalCustom, Checkbox, ButtonSwitch, InlineNotification } from '@/components';
import { RadioButtonGroup, RadioButton, CheckboxGroup, ComboBox } from '@carbon/react';
import {
    // Location as IconLocation,
    Earth as IconEarth,
    Screen as IconScreen,
} from '@carbon/react/icons';

// styles
import css from './FormWidgetClock.scss';

interface OptionItem<T extends string | number> {
    value: T;
    label: number | string;
}
const $ = getID('settings', 'clock', 'scene').get;

export interface FormWidgetClockProps {
    isOpen: boolean;
    isEdit: boolean;
    onClose(): void;
    error: Maybe<string>;

    widgetSize: null | (iField<pb.WidgetSize> & { options: Array<Exclude<pb.WidgetSize, 0>> });

    clockStyle: iField<pb.ClockWidget_ClockStyle>;
    fontStyle: iField<pb.FontStyle>;

    showDate: iField<boolean>;
    showSeconds: iField<boolean>;

    showTimezone: iField<boolean>;
    timezone: iField<string> & { options: pb.Timezone[] };

    // showWeather: iField<boolean>;
    // weatherLocation: iField<string>;

    style?: CSSProperties;
}
interface Props extends FormWidgetClockProps {
    intl: IntlShape;
}

class View extends Component<Props> {
    get #clockStyleOptions(): Array<OptionItem<pb.ClockWidget_ClockStyle>> {
        const { formatMessage } = this.props.intl;
        return [
            { value: pb.ClockWidget_ClockStyle.ANALOG_ROUND, label: formatMessage({ defaultMessage: 'Analog 1' }) },
            { value: pb.ClockWidget_ClockStyle.ANALOG_RECT, label: formatMessage({ defaultMessage: 'Analog 2' }) },
            { value: pb.ClockWidget_ClockStyle.DIGITAL, label: formatMessage({ defaultMessage: 'Digital 1' }) },
        ];
    }
    get #fontStyleOptions(): Array<OptionItem<pb.FontStyle>> {
        const { formatMessage } = this.props.intl;
        return [
            { value: pb.FontStyle.LIGHT, label: formatMessage({ defaultMessage: 'Light' }) },
            { value: pb.FontStyle.MEDIUM, label: formatMessage({ defaultMessage: 'Medium' }) },
            { value: pb.FontStyle.BOLD, label: formatMessage({ defaultMessage: 'Bold' }) },
        ];
    }

    #txt = {
        clock: this.props.intl.formatMessage({ defaultMessage: 'Clock' }),
        addScene: this.props.intl.formatMessage({ defaultMessage: 'Add Scene' }),
        editScene: this.props.intl.formatMessage({ defaultMessage: 'Edit Scene' }),
    };

    render() {
        const {
            isOpen,
            isEdit,
            onClose,
            error,

            // Main
            widgetSize,
            clockStyle,
            fontStyle,

            // Additional
            showDate,
            showSeconds,

            showTimezone,
            timezone,

            // showWeather,
            // weatherLocation,

            style,
            intl: { formatMessage },
        } = this.props;

        const form = (
            <Form className={css.root} style={style}>
                {widgetSize == null ? null : (
                    <ButtonSwitch
                        options={[
                            {
                                id: pb.WidgetSize.SMALL,
                                text: formatMessage({ defaultMessage: 'Small' }),
                                disabled: !widgetSize.options.includes(pb.WidgetSize.SMALL),
                            },
                            {
                                id: pb.WidgetSize.MEDIUM,
                                text: formatMessage({ defaultMessage: 'Medium' }),
                                disabled: !widgetSize.options.includes(pb.WidgetSize.MEDIUM),
                            },
                            {
                                id: pb.WidgetSize.LARGE,
                                text: formatMessage({ defaultMessage: 'Large' }),
                                disabled: !widgetSize.options.includes(pb.WidgetSize.LARGE),
                            },
                        ]}
                        onChange={widgetSize.onChange}
                        selectedOption={widgetSize.value}
                        disabled={widgetSize.disabled}
                        invalid={!!widgetSize.error}
                        invalidText={widgetSize.error}
                    />
                )}

                <BoundRadioGroup
                    {...clockStyle}
                    idSuffix="style"
                    labelText={formatMessage({ defaultMessage: 'Clock Style' })}
                    items={this.#clockStyleOptions}
                />
                <BoundRadioGroup
                    {...fontStyle}
                    idSuffix="font"
                    labelText={formatMessage({ defaultMessage: 'Numbers Font Style' })}
                    items={this.#fontStyleOptions}
                />

                <CheckboxGroup legendText={formatMessage({ defaultMessage: 'Additional Options' })}>
                    <BoundCheckbox
                        {...showDate}
                        idSuffix="show-date"
                        labelText={formatMessage({ defaultMessage: 'Show Date' })}
                    />
                    <BoundCheckbox
                        {...showSeconds}
                        idSuffix="show-seconds"
                        labelText={formatMessage({ defaultMessage: 'Show Seconds' })}
                    />
                    <BoundCheckbox
                        {...showTimezone}
                        idSuffix="show-timezone"
                        labelText={formatMessage({ defaultMessage: 'Show Timezone' })}
                    />
                    {/* <BoundCheckbox {...showWeather} idSuffix="show-weather" labelText={formatMessage({ defaultMessage: 'Show Weather' })} /> */}
                </CheckboxGroup>

                {showTimezone.value === true ? (
                    <BoundComboBox<string>
                        idSuffix="timezone"
                        {...timezone}
                        items={timezone.options.map(x => ({ value: x.id, label: `${x.offset} ${x.label}` }))}
                        labelText={formatMessage({ defaultMessage: 'Timezone' })}
                        decorator={<IconEarth size={20} />}
                        helperText={formatMessage({ defaultMessage: 'Location is used for Timezone.' })}
                    />
                ) : null}

                {/* showWeather.value === true ? (
                    <BoundComboBox
                        idSuffix="location"
                        {...weatherLocation}
                        items={[{ label: 'Location', value: 'Location' }]}
                        labelText={formatMessage({ defaultMessage: 'Weather Location' })}
                        decorator={<IconLocation size={20} />}
                    />
                ) : null */}

                <div className={css.note}>
                    <IconScreen size={16} />
                    <FormattedMessage
                        tagName="span"
                        defaultMessage="<b>Note</b>: Check your BMC screen to see live preview"
                        values={{ b: ch => <strong children={ch} /> }}
                    />
                </div>

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

        const verb = isEdit ? this.#txt.editScene : this.#txt.addScene;

        return (
            <ModalCustom
                id={$('dialog')}
                className={css.modal}
                selectorPrimaryFocus="input"
                // State
                size="sm"
                open={isOpen}
                // Heading
                title={this.#txt.clock}
                label={verb}
                // Cancel
                onClose={onClose}
                // Content
                children={form}
            />
        );
    }
}
export function FormWidgetClock(props: FormWidgetClockProps) {
    const intl = useIntl();
    return <View {...props} intl={intl} />;
}

interface BoundCheckboxProps extends iField<boolean> {
    idSuffix: string;
    labelText: string;
}
function BoundCheckbox(props: BoundCheckboxProps) {
    const { idSuffix, value, labelText, error, onChange, disabled } = props;
    return (
        <Checkbox
            id={$(idSuffix)}
            checked={!!value}
            label={labelText}
            disabled={disabled}
            onChange={(_, { checked }) => onChange(checked)}
            invalid={!!error}
            invalidText={error}
        />
    );
}

interface BoundComboBoxProps<T extends string | number> extends iField<T> {
    idSuffix: string;
    labelText: string;
    items: Array<OptionItem<T>>;
    decorator?: ReactNode;
    helperText?: ReactNode;
}
function BoundComboBox<T extends string | number>(props: BoundComboBoxProps<T>) {
    const { idSuffix, labelText, helperText, decorator, value, items, onChange, disabled, error } = props;

    return (
        <ComboBox<OptionItem<T>>
            id={$(idSuffix)}
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

interface BoundRadioGroupProps<T extends string | number> extends iField<T> {
    idSuffix: string;
    labelText: string;
    items: Array<OptionItem<T>>;
    decorator?: ReactNode;
    helperText?: ReactNode;
}
function BoundRadioGroup<T extends string | number>(props: BoundRadioGroupProps<T>) {
    const { idSuffix, labelText, helperText, decorator, value, items, onChange, disabled, error } = props;
    const id = $(idSuffix);

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
