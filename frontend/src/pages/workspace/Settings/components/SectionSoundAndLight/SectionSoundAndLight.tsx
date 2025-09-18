import { Component } from 'react';
import { useIntl, type IntlShape } from 'react-intl';
import { Form, type iField } from '@/lib/form';

// App
import { getID } from '../../const';
import * as pb from '@/proto';

// Components
import { Field, FieldSet, CarbonFormField } from '@/components';
import { Toggle, Slider } from '@carbon/react';

// Styles
import css from './SectionSoundAndLight.scss';

export interface SectionSoundAndLightProps {
    soundVolume: iField<pb.SoundVolume>;
    soundVolumeNight: iField<pb.SoundVolume>;
    // alarmAndNotifyVolume: iField<Integer<0, 100>>;
    ledNotifyEnabled: iField<boolean>;
}
interface Props extends SectionSoundAndLightProps {
    intl: IntlShape;
}

const $ = getID('general').get;

class View extends Component<Props> {
    #handleVolumeChange = (x: { value: number }): void => {
        const { value, onChange } = this.props.soundVolume;

        onChange(
            pb.create(pb.SoundVolumeSchema, {
                min: value?.min,
                max: value?.max,
                step: value?.step,
                value: x.value,
            }),
        );
    };
    #handleNightVolumeChange = (x: { value: number }): void => {
        const { value, onChange } = this.props.soundVolumeNight;

        onChange(
            pb.create(pb.SoundVolumeSchema, {
                min: value?.min,
                max: value?.max,
                step: value?.step,
                value: x.value,
            }),
        );
    };

    render() {
        const {
            intl,

            // Fields
            soundVolume,
            soundVolumeNight,
            // alarmAndNotifyVolume,
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
                            id={$('sound', 'volume', 'day')}
                            hideLabel
                            labelText=""
                            // Range
                            min={soundVolume.value?.min ?? 0}
                            max={soundVolume.value?.max ?? 100}
                            step={soundVolume.value?.step ?? 1}
                            // Value
                            value={soundVolume.value?.value ?? 0}
                            disabled={soundVolume.disabled}
                            onChange={this.#handleVolumeChange}
                            invalid={!!soundVolume.error}
                            invalidText={soundVolume.error}
                        />
                    </Field>

                    <Field
                        title={intl.formatMessage({ defaultMessage: 'Sound Volume in Night Mode' })}
                        disabled={soundVolumeNight.disabled}
                    >
                        <Slider
                            id={$('sound', 'volume', 'night')}
                            hideLabel
                            labelText=""
                            // Range
                            min={soundVolumeNight.value?.min ?? 0}
                            max={soundVolumeNight.value?.max ?? 100}
                            step={soundVolumeNight.value?.step ?? 1}
                            // Value
                            value={soundVolumeNight.value?.value ?? 0}
                            disabled={soundVolumeNight.disabled}
                            onChange={this.#handleNightVolumeChange}
                            invalid={!!soundVolumeNight.error}
                            invalidText={soundVolumeNight.error}
                        />
                    </Field>

                    {/* <Field
                        title={intl.formatMessage({ defaultMessage: 'Alarm and Notifications Volume' })}
                        disabled={alarmAndNotifyVolume.disabled}
                    >
                        <Slider
                            id={$('alarms', 'notify', 'volume')}
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
                    </Field> */}
                </FieldSet>

                <FieldSet title={intl.formatMessage({ defaultMessage: 'LED Notification Lights' })}>
                    <Field
                        title={intl.formatMessage({ defaultMessage: 'Enable LED Notifications' })}
                        description={intl.formatMessage({
                            defaultMessage: 'Use LED lights for notifications and alerts',
                        })}
                        disabled={ledNotifyEnabled.disabled}
                    >
                        <CarbonFormField error={ledNotifyEnabled.error}>
                            <Toggle
                                id={$('led', 'notify', 'enabled')}
                                size="md"
                                aria-invalid={!!ledNotifyEnabled.error}
                                toggled={!!ledNotifyEnabled.value}
                                onToggle={ledNotifyEnabled.onChange}
                                disabled={ledNotifyEnabled.disabled}
                            />
                        </CarbonFormField>
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
