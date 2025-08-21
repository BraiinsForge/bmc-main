import { Component, type ChangeEvent } from 'react';
import { useIntl, type IntlShape } from 'react-intl';
import { Form, type iField, type iFieldNumber, getID } from '@/lib/form';

import * as pb from '@/proto';

import { CarbonFormField, Field, FieldSet, Button } from '@/components';
import { Toggle, Slider, TextInput, TimePicker } from '@carbon/react';
import { Location } from '@carbon/react/icons';

// Styles
import css from './SectionDisplay.scss';

export interface SectionDisplayProps {
    brightness: iFieldNumber<Integer<0, 100>>;
    nightBrightness: iFieldNumber<Integer<0, 100>>;
    nightEnabled: iField<boolean>;
    nightUseLocation: iField<boolean>;
    nightLocation: iField<string>;
    onLocationDetect(): void;
    nightNotify: iField<boolean>;
    nightInterval: iField<pb.TimeInterval>;
}
interface Props extends SectionDisplayProps {
    intl: IntlShape;
}

const $ = getID('settings', 'general').get;

class View extends Component<Props> {
    #nightIntervalChange = (field: 'from' | 'to') => (e: ChangeEvent<HTMLInputElement>) => {
        const { onChange, value } = this.props.nightInterval;

        const newValue = pb.create(pb.TimeIntervalSchema, {
            from: value?.from,
            to: value?.to,
            [field]: e.target.value,
        });

        onChange?.(newValue);
    };

    render() {
        const {
            intl,

            // Fields
            brightness,

            // Night mode
            nightEnabled,
            nightBrightness,
            nightInterval,

            nightUseLocation,
            nightLocation,
            onLocationDetect,
            nightNotify,
        } = this.props;

        const isNightIntervalDisabled: boolean = nightInterval.disabled || !nightEnabled.value;
        const isNightBrightnessDisabled: boolean = nightBrightness.disabled || !nightEnabled.value;

        return (
            <Form className={css.root}>
                <FieldSet title={intl.formatMessage({ defaultMessage: 'Brigthness' })}>
                    <Field
                        title={intl.formatMessage({ defaultMessage: 'Screen Brightness' })}
                        disabled={brightness.disabled}
                    >
                        <Slider
                            id={$('brightness-day')}
                            hideLabel
                            labelText=""
                            // Range
                            stepMultiplier={10}
                            min={brightness.min ?? 0}
                            max={brightness.max ?? 0}
                            step={brightness.step ?? 1}
                            // Value
                            value={brightness.value ?? 0}
                            disabled={brightness.disabled}
                            onChange={x => brightness.onChange(x.value)}
                            invalid={!!brightness.error}
                            invalidText={brightness.error}
                        />
                    </Field>
                </FieldSet>

                <FieldSet title={intl.formatMessage({ defaultMessage: 'Night Mode' })}>
                    <Field
                        title={intl.formatMessage({ defaultMessage: 'Enable Night Mode' })}
                        description={intl.formatMessage({
                            defaultMessage:
                                'Automatically switch to red light theme after dark to preserve night vision.',
                        })}
                        disabled={nightEnabled.disabled}
                    >
                        <CarbonFormField error={nightEnabled.error} style={{ display: 'inline-block' }}>
                            <Toggle
                                id={$('night', 'enabled')}
                                size="md"
                                aria-invalid={!!nightEnabled.error}
                                toggled={!!nightEnabled.value}
                                onToggle={nightEnabled.onChange}
                                disabled={nightEnabled.disabled}
                            />
                        </CarbonFormField>
                    </Field>

                    <Field
                        title={intl.formatMessage({ defaultMessage: 'Night Mode Brightness' })}
                        disabled={isNightBrightnessDisabled}
                    >
                        <Slider
                            id={$('night', 'brightness')}
                            hideLabel
                            labelText=""
                            // Range
                            stepMultiplier={10}
                            min={brightness.min ?? 0}
                            max={brightness.max ?? 100}
                            step={brightness.step ?? 1}
                            // Value
                            value={nightBrightness.value ?? 0}
                            disabled={isNightBrightnessDisabled}
                            onChange={x => nightBrightness.onChange(x.value)}
                            invalid={!!nightBrightness.error}
                            invalidText={nightBrightness.error}
                        />
                    </Field>

                    <Field
                        title={intl.formatMessage({ defaultMessage: 'Night Mode Time Interval' })}
                        disabled={isNightIntervalDisabled}
                    >
                        <CarbonFormField error={nightInterval.error}>
                            <div className={css.timeInterval}>
                                <TimePicker
                                    id={$('night', 'interval', 'from')}
                                    placeholder="HH:MM"
                                    value={nightInterval?.value?.from ?? undefined}
                                    onChange={this.#nightIntervalChange('from')}
                                    invalid={!!nightInterval.error}
                                    disabled={isNightIntervalDisabled}
                                />
                                <div className={css.divider}>-</div>
                                <TimePicker
                                    id={$('night', 'interval', 'to')}
                                    placeholder="HH:MM"
                                    value={nightInterval?.value?.to ?? undefined}
                                    onChange={this.#nightIntervalChange('to')}
                                    invalid={!!nightInterval.error}
                                    disabled={isNightIntervalDisabled}
                                />
                            </div>
                        </CarbonFormField>
                    </Field>
                </FieldSet>

                <FieldSet title={null}>
                    <Field
                        title={intl.formatMessage({ defaultMessage: 'Use Device Location' })}
                        description={intl.formatMessage({
                            defaultMessage: 'Use location to determine sunrise and sunset times.',
                        })}
                        disabled={nightUseLocation.disabled}
                    >
                        <CarbonFormField error={nightUseLocation.error}>
                            <Toggle
                                id={$('night', 'use', 'location')}
                                size="md"
                                aria-invalid={!!nightUseLocation.error}
                                toggled={!!nightUseLocation.value}
                                onToggle={nightUseLocation.onChange}
                                disabled={nightUseLocation.disabled}
                            />
                        </CarbonFormField>
                    </Field>

                    <Field
                        title={intl.formatMessage({ defaultMessage: 'Location' })}
                        description={intl.formatMessage({ defaultMessage: 'Enter city or postal code' })}
                        disabled={nightLocation.disabled}
                    >
                        <div className={css.locationInputWrapper}>
                            <TextInput
                                id={$('night', 'location')}
                                labelText=""
                                hideLabel
                                invalid={!!nightLocation.error}
                                invalidText={nightLocation.error}
                                disabled={nightLocation.disabled}
                                value={nightLocation.value ?? ''}
                                onChange={e => nightLocation.onChange(e.target.value)}
                            />
                            <Button
                                id={$('night', 'location', 'detect')}
                                kind="secondary"
                                size="sm"
                                icon={Location}
                                children={intl.formatMessage({ defaultMessage: 'Detect' })}
                                onClick={onLocationDetect}
                            />
                        </div>
                    </Field>

                    <Field
                        title={intl.formatMessage({ defaultMessage: 'Notifications in Night Mode' })}
                        description={intl.formatMessage({
                            defaultMessage: 'Sound & LED notifications when night mode is active.',
                        })}
                        disabled={nightNotify.disabled}
                    >
                        <CarbonFormField error={nightNotify.error}>
                            <Toggle
                                id={$('night', 'notifications', 'enabled')}
                                size="md"
                                aria-invalid={!!nightNotify.error}
                                toggled={!!nightNotify.value}
                                onToggle={nightNotify.onChange}
                                disabled={nightNotify.disabled}
                            />
                        </CarbonFormField>
                    </Field>
                </FieldSet>
            </Form>
        );
    }
}

export function SectionDisplay(props: SectionDisplayProps) {
    const intl = useIntl();
    return <View {...props} intl={intl} />;
}
