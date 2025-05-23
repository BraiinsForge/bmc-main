import { Component } from 'react';
import { useIntl, type IntlShape } from 'react-intl';
import { Form, type iField, getID } from '@/lib/form';

import { Field } from '../Field';
import { FieldSet } from '../FieldSet';

import { FormField } from '@/components';
import { Toggle, Slider } from '@carbon/react';

// Styles
import css from './SectionSoundAndLight.scss';

export interface SectionSoundAndLightProps {
    soundVolume: iField<Integer<0, 100>>;
    soundVolumeNight: iField<Integer<0, 100>>;
    alarmAndNotifyVolume: iField<Integer<0, 100>>;
    ledNotifyEnabled: iField<boolean>;
}
interface Props extends SectionSoundAndLightProps {
    intl: IntlShape;
}

const $id = getID('settings', 'general');

class View extends Component<Props> {
    render() {
        const {
            intl,

            // Fields
            soundVolume,
            soundVolumeNight,
            alarmAndNotifyVolume,
            ledNotifyEnabled,
        } = this.props;

        return (
            <Form className={css.root}>
                <FieldSet title={intl.formatMessage({ defaultMessage: 'Volume' })}>
                    <Field
                        title={intl.formatMessage({ defaultMessage: 'Sound Volume' })}
                        disabled={soundVolume.disabled}
                    >
                        <Slider
                            id={$id.get('sound', 'volume', 'day')}
                            hideLabel
                            labelText=""
                            // Range
                            step={1}
                            stepMultiplier={10}
                            min={0}
                            max={100}
                            // Value
                            value={soundVolume.value ?? 0}
                            disabled={soundVolume.disabled}
                            onChange={x => soundVolume.onChange(x.value)}
                            invalid={!!soundVolume.error}
                            invalidText={soundVolume.error}
                        />
                    </Field>

                    <Field
                        title={intl.formatMessage({ defaultMessage: 'Sound Volume in Night Mode' })}
                        disabled={soundVolumeNight.disabled}
                    >
                        <Slider
                            id={$id.get('sound', 'volume', 'night')}
                            hideLabel
                            labelText=""
                            // Range
                            step={1}
                            stepMultiplier={10}
                            min={0}
                            max={100}
                            // Value
                            value={soundVolumeNight.value ?? 0}
                            disabled={soundVolumeNight.disabled}
                            onChange={x => soundVolumeNight.onChange(x.value)}
                            invalid={!!soundVolumeNight.error}
                            invalidText={soundVolumeNight.error}
                        />
                    </Field>

                    <Field
                        title={intl.formatMessage({ defaultMessage: 'Alarm and Notifications Volume' })}
                        disabled={alarmAndNotifyVolume.disabled}
                    >
                        <Slider
                            id={$id.get('alarms', 'notify', 'volume')}
                            hideLabel
                            labelText=""
                            // Range
                            step={1}
                            stepMultiplier={10}
                            min={0}
                            max={100}
                            // Value
                            value={alarmAndNotifyVolume.value ?? 0}
                            disabled={alarmAndNotifyVolume.disabled}
                            onChange={x => alarmAndNotifyVolume.onChange(x.value)}
                            invalid={!!alarmAndNotifyVolume.error}
                            invalidText={alarmAndNotifyVolume.error}
                        />
                    </Field>
                </FieldSet>

                <FieldSet title={intl.formatMessage({ defaultMessage: 'LED Notification Lights' })}>
                    <Field
                        title={intl.formatMessage({ defaultMessage: 'Enable LED Notifications' })}
                        description={intl.formatMessage({
                            defaultMessage: 'Use LED lights for notifications and alerts',
                        })}
                        disabled={ledNotifyEnabled.disabled}
                    >
                        <FormField error={ledNotifyEnabled.error}>
                            <Toggle
                                id={$id.get('led', 'notify', 'enabled')}
                                size="md"
                                aria-invalid={!!ledNotifyEnabled.error}
                                toggled={!!ledNotifyEnabled.value}
                                onToggle={ledNotifyEnabled.onChange}
                                disabled={ledNotifyEnabled.disabled}
                            />
                        </FormField>
                    </Field>
                </FieldSet>
            </Form>
        );
    }
}

export function SectionSoundAndLight(props: SectionSoundAndLightProps) {
    const intl = useIntl();
    return <View {...props} intl={intl} />;
}
