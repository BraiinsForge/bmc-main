import { Component, type ChangeEvent } from 'react';
import { useIntl, type IntlShape } from 'react-intl';
import { Form, type iField } from '@/lib/form';
import { getID } from '../../const';

import * as pb from '@/proto';

import { CarbonFormField, Field, FieldSet, Button } from '@/components';
import { Toggle, Slider, TextInput, TimePicker } from '@carbon/react';
import { Location as IconLocation, Checkmark as IconCheckmark } from '@carbon/react/icons';

// Styles
import css from './SectionDisplay.scss';

export interface SectionDisplayProps {
    brightness: iField<pb.BrightnessInfo>;

    nightEnabled: iField<boolean>;
    nightBrightness: iField<pb.BrightnessInfo>;
    nightUseLocation: iField<boolean>;

    nightLocation: iField<string>;
    onLocationDetect(): void;

    nightNotify: iField<boolean>;
    nightInterval: iField<pb.TimeInterval> & { hasChanged: boolean; onConfirm(): void };
}
interface Props extends SectionDisplayProps {
    intl: IntlShape;
}

const $ = getID('display').get;
class View extends Component<Props> {
    #brightnessChange = (x: { value: number }) => {
        const { value, onChange } = this.props.brightness;
        onChange(
            pb.create(pb.BrightnessInfoSchema, {
                value: x.value,
                min: value?.min,
                max: value?.max,
                step: value?.step,
            }),
        );
    };
    #brightnessNightChange = (x: { value: number }) => {
        const { value, onChange } = this.props.nightBrightness;
        onChange(
            pb.create(pb.BrightnessInfoSchema, {
                value: x.value,
                min: value?.min,
                max: value?.max,
                step: value?.step,
            }),
        );
    };
    #nightIntervalChange = (field: 'from' | 'to') => (e: ChangeEvent<HTMLInputElement>) => {
        const { onChange, value } = this.props.nightInterval;

        const newValue = pb.create(pb.TimeIntervalSchema, {
            from: value?.from?.trim(),
            to: value?.to?.trim(),
            [field]: e.target.value?.trim(),
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
                            min={brightness.value?.min ?? 0}
                            max={brightness.value?.max ?? 0}
                            step={brightness.value?.step ?? 1}
                            // Value
                            value={brightness.value?.value ?? 0}
                            disabled={brightness.disabled}
                            onChange={this.#brightnessChange}
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
                        <CarbonFormField error={nightEnabled.error}>
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
                            min={brightness.value?.min ?? 0}
                            max={brightness.value?.max ?? 100}
                            step={brightness.value?.step ?? 1}
                            // Value
                            value={nightBrightness.value?.value ?? 0}
                            disabled={isNightBrightnessDisabled}
                            onChange={this.#brightnessNightChange}
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
                                    invalidText={null}
                                />

                                <div className={css.divider} children="-" />

                                <TimePicker
                                    id={$('night', 'interval', 'to')}
                                    placeholder="HH:MM"
                                    value={nightInterval?.value?.to ?? undefined}
                                    onChange={this.#nightIntervalChange('to')}
                                    invalid={!!nightInterval.error}
                                    disabled={isNightIntervalDisabled}
                                    invalidText={null}
                                />

                                <Button
                                    id={$('night', 'interval', 'confirm')}
                                    disabled={isNightIntervalDisabled || !nightInterval.hasChanged}
                                    // primary[disabled] looks bad here, so we'll fake
                                    // a different disabled style by changing the kind
                                    kind={nightInterval.hasChanged ? 'primary' : 'ghost'}
                                    size="md"
                                    hasIconOnly
                                    icon={IconCheckmark}
                                    title={intl.formatMessage({ defaultMessage: 'Confirm' })}
                                    onClick={nightInterval.onConfirm}
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
                                icon={IconLocation}
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
