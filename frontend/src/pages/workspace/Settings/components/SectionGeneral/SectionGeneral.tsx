import { Component } from 'react';
import { useIntl, type IntlShape } from 'react-intl';

import type * as pb from '@/proto';
import { Form, type iField, getID } from '@/lib/form';

import { Field } from '../Field';
import { FieldSet } from '../FieldSet';

import { Button, ButtonSwitch, FormField } from '@/components';
import { Toggle, Dropdown, type DropdownProps, ComboBox, type ComboBoxProps } from '@carbon/react';
import { TemperatureCelsius, TemperatureFahrenheit } from '@carbon/react/icons';

// Styles
import css from './SectionGeneral.scss';

export enum TimeFormat {
    twelve = 12,
    twentyFour = 24,
}
export enum WeekDay {
    Monday = 1,
    Tuesday = 2,
    Wednesday = 3,
    Thursday = 4,
    Friday = 5,
    Saturday = 6,
    Sunday = 7,
}
export enum Temperature {
    C = 'C',
    F = 'F',
}
export const DateFormat = {
    DMY_DOT: 'DD.MM.YYYY',
    DMY_SLASH: 'DD/MM/YYYY',
    YDM_SLASH: 'YYYY/DD/MM',

    MDY_DOT: 'MM.DD.YYYY',
    MDY_SLASH: 'MM/DD/YYYY',
    YMD_SLASH: 'YYYY/MM/DD',
} as const;
export const NumberFormat = {
    spaceAndComma: '1 234 567,89',
    commaAndDot: '1,234,567.89',
    dotandComma: '1.234.567,89',
    spaceAndDot: '1 234 567.89',
} as const;

export interface SectionGeneralProps {
    timeFormat: iField<TimeFormat>;
    secondsInStatusbar: iField<boolean>;
    timezone: iField<pb.Timezone> & { items: ReadonlyArray<pb.Timezone> };
    dateFormat: iField<keyof typeof DateFormat>;
    firstWeekDay: iField<WeekDay>;

    temperature: iField<Temperature>;
    numberFormat: iField<keyof typeof NumberFormat>;

    onFactoryReset?(): void;
}
interface Props extends SectionGeneralProps {
    intl: IntlShape;
}

const $id = getID('settings', 'general');

class View extends Component<Props> {
    #dateFormatOptions = Object.keys(DateFormat) as (keyof typeof DateFormat)[];
    #dateFormatRender = (k: keyof typeof DateFormat): string => DateFormat[k];
    #dateFormatChange: DropdownProps<keyof typeof DateFormat>['onChange'] = x => {
        const { onChange } = this.props.dateFormat;
        if (x.selectedItem) onChange(x.selectedItem);
    };

    #numberFormatOptions = Object.keys(NumberFormat) as (keyof typeof NumberFormat)[];
    #numberFormatRender = (k: keyof typeof NumberFormat): string => NumberFormat[k];
    #numberFormatChange: DropdownProps<keyof typeof NumberFormat>['onChange'] = x => {
        const { onChange } = this.props.numberFormat;
        if (x.selectedItem) onChange(x.selectedItem);
    };

    #weekDayOptions = [
        WeekDay.Monday,
        WeekDay.Tuesday,
        WeekDay.Wednesday,
        WeekDay.Thursday,
        WeekDay.Friday,
        WeekDay.Saturday,
        WeekDay.Sunday,
    ];
    #weekDayRender = (k: WeekDay): string => {
        const { formatMessage } = this.props.intl;
        switch (k) {
            case WeekDay.Monday:
                return formatMessage({ defaultMessage: 'Monday' });
            case WeekDay.Tuesday:
                return formatMessage({ defaultMessage: 'Tuesday' });
            case WeekDay.Wednesday:
                return formatMessage({ defaultMessage: 'Wednesday' });
            case WeekDay.Thursday:
                return formatMessage({ defaultMessage: 'Thursday' });
            case WeekDay.Friday:
                return formatMessage({ defaultMessage: 'Friday' });
            case WeekDay.Saturday:
                return formatMessage({ defaultMessage: 'Saturday' });
            case WeekDay.Sunday:
                return formatMessage({ defaultMessage: 'Sunday' });
        }
    };
    #weekDayChange: DropdownProps<WeekDay>['onChange'] = x => {
        const { onChange } = this.props.firstWeekDay;
        if (x.selectedItem) onChange(x.selectedItem);
    };

    #timezoneRender = (tz: Maybe<pb.Timezone>): string => {
        if (!tz) return 'N/A';
        return `UTC${tz.offset} (${tz.label})`;
    };
    #timezoneRenderElement = (tz: pb.Timezone): ReactElement => {
        return (
            <span className={css.timezoneElement}>
                <span children={`UTC${tz.offset}`} className={css.mono} />
                <span children={`(${tz.label})`} />
            </span>
        );
    };
    #timezoneChange: ComboBoxProps<pb.Timezone>['onChange'] = x => {
        const { onChange } = this.props.timezone;
        if (x.selectedItem) onChange(x.selectedItem);
    };

    #temperatureOptions = [
        {
            id: Temperature.C,
            icon: TemperatureCelsius,
            text: this.props.intl.formatMessage({ defaultMessage: 'Celsius' }),
        },
        {
            id: Temperature.F,
            icon: TemperatureFahrenheit,
            text: this.props.intl.formatMessage({ defaultMessage: 'Fahrenheit' }),
        },
    ];

    render() {
        const {
            intl,

            // Fields
            timeFormat,
            secondsInStatusbar,
            timezone,
            dateFormat,
            firstWeekDay,

            temperature,
            numberFormat,

            onFactoryReset,
        } = this.props;

        return (
            <Form className={css.root}>
                <FieldSet title={intl.formatMessage({ defaultMessage: 'Time & Date' })}>
                    <Field title={intl.formatMessage({ defaultMessage: 'Time Format' })} disabled={timeFormat.disabled}>
                        <ButtonSwitch<TimeFormat>
                            selectedOption={timeFormat.value}
                            options={[
                                { id: TimeFormat.twelve, text: intl.formatMessage({ defaultMessage: '12-hour' }) },
                                { id: TimeFormat.twentyFour, text: intl.formatMessage({ defaultMessage: '24-hour' }) },
                            ]}
                            size="md"
                            disabled={timeFormat.disabled}
                            onChange={timeFormat.onChange}
                            invalid={!!timeFormat.error}
                            invalidText={timeFormat.error}
                        />
                    </Field>

                    <Field
                        title={intl.formatMessage({ defaultMessage: 'Show Seconds in Status Bar' })}
                        disabled={secondsInStatusbar.disabled}
                    >
                        <FormField error={secondsInStatusbar.error}>
                            <Toggle
                                id={$id.get('secondsInStatusbar')}
                                size="md"
                                aria-invalid={!!secondsInStatusbar.error}
                                toggled={!!secondsInStatusbar.value}
                                onToggle={secondsInStatusbar.onChange}
                                disabled={secondsInStatusbar.disabled}
                            />
                        </FormField>
                    </Field>

                    <Field title={intl.formatMessage({ defaultMessage: 'Timezone' })} disabled={timezone.disabled}>
                        <ComboBox<pb.Timezone>
                            id={$id.get('timezone')}
                            titleText=""
                            disabled={timezone.disabled}
                            direction="bottom"
                            items={Array.from(timezone.items)}
                            onChange={this.#timezoneChange}
                            itemToString={this.#timezoneRender}
                            itemToElement={this.#timezoneRenderElement}
                            selectedItem={timezone.value}
                            invalid={!!timezone.error}
                            invalidText={timezone.error}
                            className={css.timezoneComboBox}
                        />
                    </Field>

                    <Field title={intl.formatMessage({ defaultMessage: 'Date Format' })} disabled={dateFormat.disabled}>
                        <Dropdown<keyof typeof DateFormat>
                            id={$id.get('date-format')}
                            size="md"
                            label=""
                            titleText=""
                            hideLabel
                            items={this.#dateFormatOptions}
                            selectedItem={dateFormat.value ?? undefined}
                            onChange={this.#dateFormatChange}
                            itemToString={this.#dateFormatRender}
                            renderSelectedItem={this.#dateFormatRender}
                            disabled={dateFormat.disabled}
                            invalid={!!dateFormat.error}
                            invalidText={dateFormat.error}
                        />
                    </Field>

                    <Field
                        title={intl.formatMessage({ defaultMessage: 'First Day of the Week' })}
                        disabled={firstWeekDay.disabled}
                    >
                        <Dropdown<WeekDay>
                            id={$id.get('first-week-day')}
                            size="md"
                            label=""
                            titleText=""
                            hideLabel
                            items={this.#weekDayOptions}
                            selectedItem={firstWeekDay.value ?? undefined}
                            onChange={this.#weekDayChange}
                            itemToString={this.#weekDayRender}
                            renderSelectedItem={this.#weekDayRender}
                            disabled={firstWeekDay.disabled}
                            invalid={!!firstWeekDay.error}
                            invalidText={firstWeekDay.error}
                        />
                    </Field>
                </FieldSet>

                <FieldSet title={intl.formatMessage({ defaultMessage: 'Regional Settings' })}>
                    <Field
                        title={intl.formatMessage({ defaultMessage: 'Temperature' })}
                        disabled={temperature.disabled}
                    >
                        <ButtonSwitch<Temperature>
                            id={$id.get('temperature')}
                            size="md"
                            selectedOption={temperature.value}
                            options={this.#temperatureOptions}
                            disabled={temperature.disabled}
                            onChange={temperature.onChange}
                            invalid={!!temperature.error}
                            invalidText={temperature.error}
                        />
                    </Field>

                    <Field
                        title={intl.formatMessage({ defaultMessage: 'Number Format' })}
                        disabled={numberFormat.disabled}
                    >
                        <Dropdown<keyof typeof NumberFormat>
                            size="md"
                            label=""
                            titleText=""
                            hideLabel
                            id={$id.get('number-format')}
                            items={this.#numberFormatOptions}
                            selectedItem={numberFormat.value ?? undefined}
                            onChange={this.#numberFormatChange}
                            itemToString={this.#numberFormatRender}
                            renderSelectedItem={this.#numberFormatRender}
                            disabled={numberFormat.disabled}
                            invalid={!!numberFormat.error}
                            invalidText={numberFormat.error}
                        />
                    </Field>
                </FieldSet>

                <FieldSet title={intl.formatMessage({ defaultMessage: 'Factory Reset' })}>
                    <Field
                        title={intl.formatMessage({ defaultMessage: 'Reset to Factory Defaults' })}
                        description={intl.formatMessage({
                            defaultMessage:
                                'Warning: This will delete all your custom configurations and display scenes.',
                        })}
                        disabled={!onFactoryReset}
                    >
                        <Button
                            kind="secondary"
                            children={intl.formatMessage({ defaultMessage: 'Reset to Defaults' })}
                            onClick={onFactoryReset}
                        />
                    </Field>
                </FieldSet>
            </Form>
        );
    }
}

export function SectionGeneral(props: SectionGeneralProps) {
    const intl = useIntl();
    return <View {...props} intl={intl} />;
}
