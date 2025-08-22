import { Component } from 'react';
import { type IntlShape, useIntl } from 'react-intl';

import { Form, getID, type iField } from '@/lib/form';
import * as pb from '@/proto';

// Components
import { Layout } from '../Layout';
import { ButtonSwitch, FieldSet, Field, LogoHeader, Tooltip, Button } from '@/components';
import {
    ComboBox,
    type ComboBoxProps,
    Dropdown,
    type DropdownProps,
    ProgressIndicator,
    ProgressStep,
    PasswordInput,
    Toggle,
} from '@carbon/react';
import { Information as IconInfo } from '@carbon/react/icons';

// Styles
import css from './Setup.scss';

type TimeFormat = Exclude<pb.TimeFormat, 0>;
type DateFormat = Exclude<pb.DateFormat, 0>;
type NumberFormat = Exclude<pb.NumberFormat, 0>;

export interface SetupProps {
    timeFormat: iField<TimeFormat>;
    timezone: iField<pb.Timezone> & { items: ReadonlyArray<pb.Timezone> };
    dateFormat: iField<DateFormat>;
    numberFormat: iField<NumberFormat>;

    password1: iField<string>;
    password2: iField<string>;

    dataCollection: iField<boolean>;

    onSubmit(): void;
    submitDisabled?: boolean;
}
interface Props extends SetupProps {
    intl: IntlShape;
}

const $ = getID('initial-setup-profile').get;
class View extends Component<Props> {
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

    #dateFormatChange: DropdownProps<DateFormat>['onChange'] = x => {
        const { onChange } = this.props.dateFormat;
        if (x.selectedItem) onChange(x.selectedItem);
    };
    #numberFormatChange: DropdownProps<NumberFormat>['onChange'] = x => {
        const { onChange } = this.props.numberFormat;
        if (x.selectedItem) onChange(x.selectedItem);
    };

    render() {
        const {
            intl: { formatMessage },
            onSubmit,
            submitDisabled,

            timeFormat,
            timezone,
            dateFormat,
            numberFormat,

            password1,
            password2,

            dataCollection,
        } = this.props;

        return (
            <Layout
                header={<LogoHeader style={{ width: 'auto', height: 18 }} />}
                footer={[
                    <span key="a" />,
                    <Button
                        id={$('save-and-continue')}
                        key="b"
                        kind="primary"
                        disabled={submitDisabled}
                        onClick={onSubmit}
                        children={formatMessage({ defaultMessage: 'Save and Continue' })}
                    />,
                ]}
                className={css.layout}
            >
                <ProgressIndicator currentIndex={1} className={css.progress}>
                    <ProgressStep label="Wifi Settings" />
                    <ProgressStep label="Initial Setup" className={css.disabledTab} />
                </ProgressIndicator>

                <h1 className={css.title} children={formatMessage({ defaultMessage: 'Device Setup' })} />
                <p
                    className={css.note}
                    children={formatMessage({
                        defaultMessage:
                            'Configure essential settings like time, network, and access to prepare your clock for use.',
                    })}
                />

                <Form className={css.form}>
                    <FieldSet title={formatMessage({ defaultMessage: 'Time, Date and Regional Settings' })}>
                        <Field
                            variant="light"
                            title={formatMessage({ defaultMessage: 'Time Format' })}
                            disabled={timeFormat.disabled}
                        >
                            <ButtonSwitch<TimeFormat>
                                id={$('time-format')}
                                selectedOption={timeFormat.value}
                                options={[
                                    {
                                        id: pb.TimeFormat.TIME_FORMAT_12_HOUR,
                                        text: formatMessage({ defaultMessage: '12-Hour' }),
                                    },
                                    {
                                        id: pb.TimeFormat.TIME_FORMAT_24_HOUR,
                                        text: formatMessage({ defaultMessage: '24-Hour' }),
                                    },
                                ]}
                                onChange={timeFormat.onChange}
                                disabled={timeFormat.disabled}
                                invalid={!!timeFormat.error}
                                invalidText={timeFormat.error}
                            />
                        </Field>

                        <Field
                            variant="light"
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
                            variant="light"
                            title={formatMessage({ defaultMessage: 'Date Format' })}
                            disabled={dateFormat.disabled}
                        >
                            <Dropdown<DateFormat>
                                id={$('date-format')}
                                size="md"
                                label=""
                                titleText=""
                                hideLabel
                                items={pb.dateFormatOptions}
                                selectedItem={dateFormat.value ?? undefined}
                                onChange={this.#dateFormatChange}
                                itemToString={x => pb.dateFormatToString(x) ?? 'N/A'}
                                renderSelectedItem={x => pb.dateFormatToString(x) ?? 'N/A'}
                                disabled={dateFormat.disabled}
                                invalid={!!dateFormat.error}
                                invalidText={dateFormat.error}
                            />
                        </Field>

                        <Field
                            variant="light"
                            title={formatMessage({ defaultMessage: 'Number Format' })}
                            disabled={numberFormat.disabled}
                        >
                            <Dropdown<NumberFormat>
                                size="md"
                                label=""
                                titleText=""
                                hideLabel
                                id={$('number-format')}
                                items={pb.numberFormatOptions}
                                selectedItem={numberFormat.value ?? undefined}
                                onChange={this.#numberFormatChange}
                                itemToString={x => pb.numberFormatToString(x) ?? 'N/A'}
                                renderSelectedItem={x => pb.numberFormatToString(x) ?? 'N/A'}
                                disabled={numberFormat.disabled}
                                invalid={!!numberFormat.error}
                                invalidText={numberFormat.error}
                            />
                        </Field>
                    </FieldSet>

                    <FieldSet
                        title={formatMessage({ defaultMessage: 'Password (optional)' })}
                        description={formatMessage({
                            defaultMessage:
                                "If you forget this password, you'll need to reset the clocks to regain access.",
                        })}
                    >
                        <Field
                            variant="light"
                            title={
                                <Tooltip
                                    render={ref => (
                                        <span ref={ref} className={css.withIcon}>
                                            <span children={formatMessage({ defaultMessage: 'Password' })} />
                                            <IconInfo size={14} />
                                        </span>
                                    )}
                                    content={formatMessage({
                                        defaultMessage: 'Password must be at least 6 characters long',
                                    })}
                                    placement="top"
                                />
                            }
                            disabled={password1.disabled}
                        >
                            <PasswordInput
                                id={$('password-1')}
                                hideLabel
                                labelText={null}
                                value={password1.value ?? ''}
                                onChange={e => password1.onChange(e.target.value)}
                                disabled={password1.disabled}
                                invalid={!!password1.error}
                                invalidText={password1.error}
                            />
                        </Field>

                        <Field
                            variant="light"
                            title={formatMessage({ defaultMessage: 'Password Repeat' })}
                            disabled={password2.disabled}
                        >
                            <PasswordInput
                                id={$('password-2')}
                                hideLabel
                                labelText={null}
                                value={password2.value ?? ''}
                                onChange={e => password2.onChange(e.target.value)}
                                disabled={password2.disabled}
                                invalid={!!password2.error}
                                invalidText={password2.error}
                            />
                        </Field>
                    </FieldSet>

                    <FieldSet title={formatMessage({ defaultMessage: 'Usage Data' })}>
                        <Field
                            variant="light"
                            title={formatMessage({ defaultMessage: 'Data Collection' })}
                            description={formatMessage({
                                defaultMessage: 'Allow anonymous data collection to improve the product',
                            })}
                            disabled={dataCollection.disabled}
                        >
                            <Toggle
                                id={$('data-collection')}
                                toggled={!!dataCollection.value}
                                onToggle={dataCollection.onChange}
                                disabled={dataCollection.disabled}
                            />
                        </Field>
                    </FieldSet>
                </Form>
            </Layout>
        );
    }
}

export function Setup(props: SetupProps) {
    const intl = useIntl();
    return <View {...props} intl={intl} />;
}
