import { Component } from 'react';
import { useIntl, type IntlShape } from 'react-intl';
import { Form, type iField, getID } from '@/lib/form';

// Components
import { Checkbox } from '@/components';
import { Location as IconLocation, Earth as IconEarth } from '@carbon/react/icons';
import { RadioButtonGroup, RadioButton, CheckboxGroup, ComboBox } from '@carbon/react';

// styles
import css from './FormSceneClock.scss';

export enum ClockStyle {
    analog1 = 'analog1',
    analog2 = 'analog2',
    digital1 = 'digital1',
    digital2 = 'digital2',
}
export enum FontStyle {
    light = 'light',
    medium = 'medium',
    bold = 'bold',
}

interface OptionItem {
    value: string;
    label: string;
}
interface LocationItem {
    value: string;
    label: string;
}

const $id = getID('settings', 'clock', 'scene');

export interface FormSceneClockProps {
    clockStyle: iField<ClockStyle>;
    fontStyle: iField<FontStyle>;

    showDate: iField<boolean>;
    showSeconds: iField<boolean>;
    showTimezone: iField<boolean>;
    showWeather: iField<boolean>;

    timezone: iField<string>;
    weatherLocation: iField<string>;

    style?: CSSProperties;
}
interface Props extends FormSceneClockProps {
    intl: IntlShape;
}

class View extends Component<Props> {
    get #clockStyleOptions(): OptionItem[] {
        const { formatMessage } = this.props.intl;
        return [
            { value: ClockStyle.analog1, label: formatMessage({ defaultMessage: 'Analog 1' }) },
            { value: ClockStyle.analog2, label: formatMessage({ defaultMessage: 'Analog 2' }) },
            { value: ClockStyle.digital1, label: formatMessage({ defaultMessage: 'Digital 1' }) },
            { value: ClockStyle.digital2, label: formatMessage({ defaultMessage: 'Digital 2' }) },
        ];
    }
    get #fontStyleOptions(): OptionItem[] {
        const { formatMessage } = this.props.intl;
        return [
            { value: FontStyle.light, label: formatMessage({ defaultMessage: 'Light' }) },
            { value: FontStyle.medium, label: formatMessage({ defaultMessage: 'Medium' }) },
            { value: FontStyle.bold, label: formatMessage({ defaultMessage: 'Bold' }) },
        ];
    }

    render() {
        const {
            // Main
            clockStyle,
            fontStyle,

            // Additional
            showDate,
            showSeconds,
            showTimezone,
            showWeather,

            timezone,
            weatherLocation,

            style,
            intl: { formatMessage },
        } = this.props;

        return (
            <Form className={css.root} style={style}>
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
                    <BoundCheckbox
                        {...showWeather}
                        idSuffix="show-weather"
                        labelText={formatMessage({ defaultMessage: 'Show Weather' })}
                    />
                </CheckboxGroup>

                {showTimezone.value === true ? (
                    <BoundComboBox
                        idSuffix="timezone"
                        {...timezone}
                        // FIXME: Format and real data?
                        items={[
                            { value: 'America/New_York', label: 'New York (UTC-5)' },
                            { value: 'Europe/London', label: 'London (UTC+0)' },
                            { value: 'Europe/Paris', label: 'Paris (UTC+1)' },
                            { value: 'Asia/Tokyo', label: 'Tokyo (UTC+9)' },
                            { value: 'Asia/Dubai', label: 'Dubai (UTC+4)' },
                            { value: 'Australia/Sydney', label: 'Sydney (UTC+11)' },
                            { value: 'Pacific/Auckland', label: 'Auckland (UTC+13)' },
                            { value: 'Asia/Singapore', label: 'Singapore (UTC+8)' },
                            { value: 'Europe/Moscow', label: 'Moscow (UTC+3)' },
                            { value: 'America/Los_Angeles', label: 'Los Angeles (UTC-8)' },
                            { value: 'Asia/Shanghai', label: 'Shanghai (UTC+8)' },
                            { value: 'Europe/Berlin', label: 'Berlin (UTC+1)' },
                            { value: 'Asia/Hong_Kong', label: 'Hong Kong (UTC+8)' },
                            { value: 'America/Chicago', label: 'Chicago (UTC-6)' },
                            { value: 'Asia/Seoul', label: 'Seoul (UTC+9)' },
                        ]}
                        // FIXME: Format and real data?
                        labelText={formatMessage({ defaultMessage: 'Timezone' })}
                        decorator={<IconEarth size={20} />}
                        helperText={formatMessage({
                            defaultMessage: 'Location is used for Timezone and Weather data.',
                        })}
                    />
                ) : null}

                {showWeather.value === true ? (
                    <BoundComboBox
                        idSuffix="location"
                        {...weatherLocation}
                        // FIXME: Format and real data?
                        items={[{ label: 'Location', value: 'Location' }]}
                        // FIXME: Format and real data?
                        labelText={formatMessage({ defaultMessage: 'Weather Location' })}
                        decorator={<IconLocation size={20} />}
                        helperText={formatMessage({
                            defaultMessage: 'Location is used for Timezone and Weather data.',
                        })}
                    />
                ) : null}
            </Form>
        );
    }
}
export function FormSceneClock(props: FormSceneClockProps) {
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
            id={$id.get(idSuffix)}
            checked={!!value}
            label={labelText}
            disabled={disabled}
            onChange={(_, { checked }) => onChange(checked)}
            invalid={!!error}
            invalidText={error}
        />
    );
}

interface BoundComboBoxProps extends iField<string> {
    idSuffix: string;
    labelText: string;
    items: OptionItem[];
    decorator?: ReactNode;
    helperText?: ReactNode;
}
function BoundComboBox(props: BoundComboBoxProps) {
    const { idSuffix, labelText, helperText, decorator, value, items, onChange, disabled, error } = props;

    return (
        <ComboBox<LocationItem>
            id={$id.get(idSuffix)}
            className={css.comboBox}
            onChange={x => onChange(x.selectedItem?.value ?? '')}
            itemToString={x => x?.label ?? 'N/A'}
            // FIXME: Data source?
            items={items}
            // FIXME: Format and real data?
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

interface BoundRadioGroupProps extends iField<string> {
    idSuffix: string;
    labelText: string;
    items: OptionItem[];
    decorator?: ReactNode;
    helperText?: ReactNode;
}
function BoundRadioGroup(props: BoundRadioGroupProps) {
    const { idSuffix, labelText, helperText, decorator, value, items, onChange, disabled, error } = props;
    const id = $id.get(idSuffix);

    return (
        <RadioButtonGroup
            id={id}
            name={id}
            value={value ?? undefined}
            legendText={labelText}
            children={items.map(x => (
                <RadioButton key={x.value} value={x.value} labelText={x.label} checked={value === x.value} />
            ))}
            onChange={v => onChange(v as string)}
            invalid={!!error}
            invalidText={error}
            helperText={helperText}
            decorator={decorator}
            disabled={disabled}
        />
    );
}
