import { Component } from 'react';
import { useIntl, type IntlShape } from 'react-intl';
import { Form, type iField, getID } from '@/lib/form';

import { CarbonFormField, Field, FieldSet, Button } from '@/components';
import { Toggle, Slider, TextInput } from '@carbon/react';
import { Location } from '@carbon/react/icons';

// Styles
import css from './SectionDisplay.scss';

export interface SectionDisplayProps {
    brightnessDay: iField<Integer<0, 100>>;
    nightBrightness: iField<Integer<0, 100>>;
    nightEnabled: iField<boolean>;
    nightUseLocation: iField<boolean>;
    nightLocation: iField<string>;
    onLocationDetect(): void;
    nightNotify: iField<boolean>;
}
interface Props extends SectionDisplayProps {
    intl: IntlShape;
}

const $id = getID('settings', 'general');

class View extends Component<Props> {
    render() {
        const {
            intl,

            // Fields
            brightnessDay,
            nightBrightness,

            // Night mode
            nightEnabled,
            nightUseLocation,
            nightLocation,
            onLocationDetect,
            nightNotify,
        } = this.props;

        return (
            <Form className={css.root}>
                <FieldSet title={intl.formatMessage({ defaultMessage: 'Brigthness' })}>
                    <Field
                        title={intl.formatMessage({ defaultMessage: 'Screen Brightness' })}
                        disabled={brightnessDay.disabled}
                    >
                        <Slider
                            id={$id.get('brightness-day')}
                            hideLabel
                            labelText=""
                            // Range
                            step={1}
                            stepMultiplier={10}
                            min={0}
                            max={100}
                            // Value
                            value={brightnessDay.value ?? 0}
                            disabled={brightnessDay.disabled}
                            onChange={x => brightnessDay.onChange(x.value)}
                            invalid={!!brightnessDay.error}
                            invalidText={brightnessDay.error}
                        />
                    </Field>

                    <Field
                        title={intl.formatMessage({ defaultMessage: 'Night Mode Brightness' })}
                        disabled={nightBrightness.disabled}
                    >
                        <Slider
                            id={$id.get('brightness-night')}
                            hideLabel
                            labelText=""
                            // Range
                            step={1}
                            stepMultiplier={10}
                            min={0}
                            max={100}
                            // Value
                            value={nightBrightness.value ?? 0}
                            disabled={nightBrightness.disabled}
                            onChange={x => nightBrightness.onChange(x.value)}
                            invalid={!!nightBrightness.error}
                            invalidText={nightBrightness.error}
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
                                id={$id.get('night', 'enabled')}
                                size="md"
                                aria-invalid={!!nightEnabled.error}
                                toggled={!!nightEnabled.value}
                                onToggle={nightEnabled.onChange}
                                disabled={nightEnabled.disabled}
                            />
                        </CarbonFormField>
                    </Field>

                    <Field
                        title={intl.formatMessage({ defaultMessage: 'Use Device Location' })}
                        description={intl.formatMessage({
                            defaultMessage: 'Use location to determine sunrise and sunset times.',
                        })}
                        disabled={nightUseLocation.disabled}
                    >
                        <CarbonFormField error={nightUseLocation.error}>
                            <Toggle
                                id={$id.get('night', 'use', 'location')}
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
                                id={$id.get('night', 'location')}
                                labelText=""
                                hideLabel
                                invalid={!!nightLocation.error}
                                invalidText={nightLocation.error}
                                disabled={nightLocation.disabled}
                                value={nightLocation.value ?? ''}
                                onChange={e => nightLocation.onChange(e.target.value)}
                            />
                            <Button
                                id={$id.get('night', 'location', 'detect')}
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
                                id={$id.get('night', 'enabled')}
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
