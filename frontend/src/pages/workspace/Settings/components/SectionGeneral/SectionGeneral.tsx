import { Component } from 'react';
import { useIntl, type IntlShape } from 'react-intl';

import * as pb from '@/proto';
import { getID } from '../../const';
import { Form, type iField } from '@/lib/form';
import AppContext, { type AppContextType } from '@/context';

import {
    Field,
    FieldSet,
    // CarbonFormField,
    Button,
    ButtonSwitch,
    // type ButtonSwitchItem,
} from '@/components';
import {
    // Toggle,
    Dropdown,
    type DropdownProps,
    ComboBox,
    type ComboBoxProps,
} from '@carbon/react';

// Styles
import css from './SectionGeneral.scss';

export interface SectionGeneralProps {
    timeFormat: iField<pb.TimeFormat>;
    // secondsInStatusbar: iField<boolean>;
    timezone: iField<pb.Timezone> & { items: ReadonlyArray<pb.Timezone> };
    dateFormat: iField<pb.DateFormat>;
    firstWeekDay: iField<pb.Weekday>;

    // Regional settings
    // temperatureUnits: iField<pb.TemperatureUnit>;
    numberFormat: iField<pb.NumberFormat>;

    // System actions
    onFactoryReset(): void;
    onSystemReboot(): void;

    // Data collection
    // usageData: iField<boolean>;
}
interface Props extends SectionGeneralProps {
    intl: IntlShape;
}

const $ = getID('general').get;

class View extends Component<Props> {
    static contextType = AppContext;
    declare context: AppContextType;

    #dateFormatChange: DropdownProps<pb.DateFormat>['onChange'] = x => {
        const { onChange } = this.props.dateFormat;
        if (x.selectedItem) onChange(x.selectedItem);
    };

    #numberFormatChange: DropdownProps<pb.NumberFormat>['onChange'] = x => {
        const { onChange } = this.props.numberFormat;
        if (x.selectedItem) onChange(x.selectedItem);
    };
    #numberFormatToString = (x: null | pb.NumberFormat): string => {
        return pb.numberFormatToString(x) ?? 'N/A';
    };

    // #temperatureOptions = Array.from(pb.temperatureUnitOptions.entries()).map<ButtonSwitchItem<pb.TemperatureUnit>>(
    //     ([key, Icon]) => ({
    //         id: key,
    //         text: pb.temperatureUnitToString(this.props.intl, key) ?? 'N/A',
    //         icon: Icon,
    //     }),
    // );

    #weekDayChange: DropdownProps<pb.Weekday>['onChange'] = x => {
        const { onChange } = this.props.firstWeekDay;
        if (x.selectedItem) onChange(x.selectedItem);
    };
    #weekDayToString = (x: null | pb.Weekday): string => {
        return pb.weekdayToString(this.props.intl, x, true) ?? 'N/A';
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
    #reboot = async (): Promise<void> => {
        const { intl, onSystemReboot } = this.props;
        const { confirm } = this.context;

        const response: boolean = await confirm({
            danger: true,
            title: intl.formatMessage({ defaultMessage: 'Reboot Device' }),
            message: intl.formatMessage({ defaultMessage: 'Do you really want to reboot the device?' }),
            confirmLabel: intl.formatMessage({ defaultMessage: 'Reboot' }),
        });
        if (response) onSystemReboot();
    };

    render() {
        const {
            // Fields
            timeFormat,
            // secondsInStatusbar,
            timezone,
            dateFormat,
            firstWeekDay,

            // temperatureUnits,
            numberFormat,
            // usageData,

            // HOC
            intl,
        } = this.props;
        const { formatMessage } = intl;

        return (
            <Form className={css.root}>
                <FieldSet title={formatMessage({ defaultMessage: 'Time & Date' })}>
                    <Field
                        variant="dark"
                        title={formatMessage({ defaultMessage: 'Time Format' })}
                        disabled={timeFormat.disabled}
                    >
                        <ButtonSwitch<pb.TimeFormat>
                            selectedOption={timeFormat.value}
                            options={[
                                {
                                    id: pb.TimeFormat.TIME_FORMAT_12_HOUR,
                                    text: formatMessage({ defaultMessage: '12-hour' }),
                                },
                                {
                                    id: pb.TimeFormat.TIME_FORMAT_24_HOUR,
                                    text: formatMessage({ defaultMessage: '24-hour' }),
                                },
                            ]}
                            size="md"
                            disabled={timeFormat.disabled}
                            onChange={timeFormat.onChange}
                            invalid={!!timeFormat.error}
                            invalidText={timeFormat.error}
                        />
                    </Field>

                    {/* <Field
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
                    </Field> */}

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
                        <Dropdown<null | pb.DateFormat>
                            id={$('date-format')}
                            size="md"
                            label=""
                            titleText=""
                            hideLabel
                            items={pb.dateFormatOptions}
                            selectedItem={dateFormat.value}
                            onChange={this.#dateFormatChange}
                            itemToString={pb.dateFormatToString}
                            renderSelectedItem={pb.dateFormatToString}
                            disabled={dateFormat.disabled}
                            invalid={!!dateFormat.error}
                            invalidText={dateFormat.error}
                        />
                    </Field>

                    <Field
                        title={formatMessage({ defaultMessage: 'First Day of the Week' })}
                        disabled={firstWeekDay.disabled}
                    >
                        <Dropdown<null | pb.Weekday>
                            id={$('first-week-day')}
                            size="md"
                            label=""
                            titleText=""
                            hideLabel
                            items={pb.weekdayOptionsAll}
                            selectedItem={firstWeekDay.value}
                            onChange={this.#weekDayChange}
                            itemToString={this.#weekDayToString}
                            renderSelectedItem={this.#weekDayToString}
                            disabled={firstWeekDay.disabled}
                            invalid={!!firstWeekDay.error}
                            invalidText={firstWeekDay.error}
                        />
                    </Field>
                </FieldSet>

                <FieldSet title={formatMessage({ defaultMessage: 'Regional Settings' })}>
                    {/*
                    <Field
                        variant="dark"
                        title={formatMessage({ defaultMessage: 'Temperature' })}
                        disabled={temperatureUnits.disabled}
                    >
                        <ButtonSwitch<pb.TemperatureUnit>
                            id={$('temperature')}
                            size="md"
                            selectedOption={temperatureUnits.value}
                            options={this.#temperatureOptions}
                            disabled={temperatureUnits.disabled}
                            onChange={temperatureUnits.onChange}
                            invalid={!!temperatureUnits.error}
                            invalidText={temperatureUnits.error}
                        />
                    </Field>
                    */}

                    <Field
                        variant="dark"
                        title={formatMessage({ defaultMessage: 'Number Format' })}
                        disabled={numberFormat.disabled}
                    >
                        <Dropdown<null | pb.NumberFormat>
                            size="md"
                            label=""
                            titleText=""
                            hideLabel
                            id={$('number-format')}
                            items={pb.numberFormatOptions}
                            selectedItem={numberFormat.value}
                            onChange={this.#numberFormatChange}
                            itemToString={this.#numberFormatToString}
                            renderSelectedItem={this.#numberFormatToString}
                            disabled={numberFormat.disabled}
                            invalid={!!numberFormat.error}
                            invalidText={numberFormat.error}
                        />
                    </Field>
                </FieldSet>

                <FieldSet title={formatMessage({ defaultMessage: 'System Actions' })}>
                    <Field
                        title={formatMessage({ defaultMessage: 'Reset to Factory Defaults' })}
                        description={formatMessage({
                            defaultMessage:
                                'Warning: This will delete all your custom configurations and display scenes.',
                        })}
                    >
                        <Button
                            id={$('factory-reset')}
                            kind="secondary"
                            children={formatMessage({ defaultMessage: 'Reset to Defaults' })}
                            onClick={this.#reset}
                        />
                    </Field>

                    <Field title={formatMessage({ defaultMessage: 'Reboot Device' })}>
                        <Button
                            id={$('system-reboot')}
                            kind="secondary"
                            children={formatMessage({ defaultMessage: 'Reboot' })}
                            onClick={this.#reboot}
                        />
                    </Field>
                </FieldSet>

                {/*}
                <FieldSet title={formatMessage({ defaultMessage: 'Usage Data' })}>
                    <Field
                        title={formatMessage({ defaultMessage: 'Data Collection' })}
                        description={formatMessage({
                            defaultMessage: 'Allow anonymous usage data collection to improve the product',
                        })}
                        disabled={usageData.disabled}
                    >
                        <CarbonFormField error={usageData.error}>
                            <Toggle
                                id={$('data-collection')}
                                size="md"
                                aria-invalid={!!usageData.error}
                                toggled={!!usageData.value}
                                onToggle={usageData.onChange}
                                disabled={usageData.disabled}
                            />
                        </CarbonFormField>
                    </Field>
                </FieldSet>
                */}
            </Form>
        );
    }
}

export function SectionGeneral(props: SectionGeneralProps) {
    const intl = useIntl();
    return <View {...props} intl={intl} />;
}
