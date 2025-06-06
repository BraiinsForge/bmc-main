import { Component } from 'react';
import { useIntl, type IntlShape } from 'react-intl';

import * as pb from '@/proto';
import AppContext, { type AppContextType } from '@/context';
import { Form, type iField, getID } from '@/lib/form';

import { Field, FieldSet, CarbonFormField, Button, ButtonSwitch } from '@/components';
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

    onFactoryReset(): void;
}
interface Props extends SectionGeneralProps {
    intl: IntlShape;
}

const $ = getID('settings', 'general').get;

class View extends Component<Props> {
    static contextType = AppContext;
    declare context: AppContextType;

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

    #reset = async (): Promise<void> => {
        const { intl, onFactoryReset } = this.props;
        const { confirm } = this.context;

        const response: boolean = await confirm({
            danger: true,
            title: intl.formatMessage({ defaultMessage: 'Factory reset' }),
            message: intl.formatMessage({
                defaultMessage: 'Do you really want to reset the device to factory settings?',
            }),
            confirmLabel: intl.formatMessage({ defaultMessage: 'Reset' }),
        });
        if (response) onFactoryReset();
    };

    render() {
        const {
            intl: { formatMessage },

            // Fields
            timeFormat,
            secondsInStatusbar,
            timezone,
            dateFormat,
            firstWeekDay,

            temperature,
            numberFormat,
        } = this.props;

        return (
            <Form className={css.root}>
                <FieldSet title={formatMessage({ defaultMessage: 'Time & Date' })}>
                    <Field
                        variant="dark"
                        title={formatMessage({ defaultMessage: 'Time Format' })}
                        disabled={timeFormat.disabled}
                    >
                        <ButtonSwitch<TimeFormat>
                            selectedOption={timeFormat.value}
                            options={[
                                { id: TimeFormat.twelve, text: formatMessage({ defaultMessage: '12-hour' }) },
                                { id: TimeFormat.twentyFour, text: formatMessage({ defaultMessage: '24-hour' }) },
                            ]}
                            size="md"
                            disabled={timeFormat.disabled}
                            onChange={timeFormat.onChange}
                            invalid={!!timeFormat.error}
                            invalidText={timeFormat.error}
                        />
                    </Field>

                    <Field
                        title={formatMessage({ defaultMessage: 'Show Seconds in Status Bar' })}
                        disabled={secondsInStatusbar.disabled}
                    >
                        <CarbonFormField error={secondsInStatusbar.error}>
                            <Toggle
                                id={$('secondsInStatusbar')}
                                size="md"
                                aria-invalid={!!secondsInStatusbar.error}
                                toggled={!!secondsInStatusbar.value}
                                onToggle={secondsInStatusbar.onChange}
                                disabled={secondsInStatusbar.disabled}
                            />
                        </CarbonFormField>
                    </Field>

                    <Field
                        variant="dark"
                        title={formatMessage({ defaultMessage: 'Timezone' })}
                        disabled={timezone.disabled}
                    >
                        <ComboBox<pb.Timezone>
                            id={$('timezone')}
                            titleText=""
                            disabled={timezone.disabled}
                            direction="bottom"
                            items={Array.from(timezone.items)}
                            onChange={this.#timezoneChange}
                            itemToString={pb.renderTimezone}
                            itemToElement={this.#timezoneRenderElement}
                            selectedItem={timezone.value}
                            invalid={!!timezone.error}
                            invalidText={timezone.error}
                            className={css.timezoneComboBox}
                        />
                    </Field>

                    <Field
                        variant="dark"
                        title={formatMessage({ defaultMessage: 'Date Format' })}
                        disabled={dateFormat.disabled}
                    >
                        <Dropdown<keyof typeof DateFormat>
                            id={$('date-format')}
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
                        title={formatMessage({ defaultMessage: 'First Day of the Week' })}
                        disabled={firstWeekDay.disabled}
                    >
                        <Dropdown<WeekDay>
                            id={$('first-week-day')}
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

                <FieldSet title={formatMessage({ defaultMessage: 'Regional Settings' })}>
                    <Field
                        variant="dark"
                        title={formatMessage({ defaultMessage: 'Temperature' })}
                        disabled={temperature.disabled}
                    >
                        <ButtonSwitch<Temperature>
                            id={$('temperature')}
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
                        variant="dark"
                        title={formatMessage({ defaultMessage: 'Number Format' })}
                        disabled={numberFormat.disabled}
                    >
                        <Dropdown<keyof typeof NumberFormat>
                            size="md"
                            label=""
                            titleText=""
                            hideLabel
                            id={$('number-format')}
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

                <FieldSet title={formatMessage({ defaultMessage: 'Factory Reset' })}>
                    <Field
                        title={formatMessage({ defaultMessage: 'Reset to Factory Defaults' })}
                        description={formatMessage({
                            defaultMessage:
                                'Warning: This will delete all your custom configurations and display scenes.',
                        })}
                    >
                        <Button
                            kind="secondary"
                            children={formatMessage({ defaultMessage: 'Reset to Defaults' })}
                            onClick={this.#reset}
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
