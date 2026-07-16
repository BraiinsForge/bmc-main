// Copyright (C) 2025  Braiins Systems s.r.o.
// Copyright (C) 2026  Braiins Forge s.r.o.
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
//
// Braiins Systems s.r.o. and Braiins Forge s.r.o. each reserve the right
// to grant any party a license to this program, or any part thereof,
// under any terms, and such a grant shall be considered distinct from
// the grant above.

import { Component } from 'react';
import { useIntl, type IntlShape } from 'react-intl';
import { Form, type iField } from '@/lib/form';
import { handleSliderParentKeyDownCapture } from '@/lib/carbon';

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
    bootSoundEnabled: iField<boolean>;

    // alarmAndNotifyVolume: iField<Integer<0, 100>>;

    ledNotifyEnabled: iField<boolean>;
    ledNotifyEnabledNight: iField<boolean>;
}
interface Props extends SectionSoundAndLightProps {
    intl: IntlShape;
}

const $ = getID('general').get;

class View extends Component<Props> {
    #handleVolumeChange = (x: { value: number }): void => {
        const { value, onChange } = this.props.soundVolume;

        onChange?.(
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

        onChange?.(
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
            bootSoundEnabled,
            // alarmAndNotifyVolume,
            ledNotifyEnabled,
            ledNotifyEnabledNight,
        } = this.props;

        return (
            <Form className={css.root}>
                <FieldSet title={intl.formatMessage({ defaultMessage: 'Volume' })}>
                    <Field
                        title={intl.formatMessage({ defaultMessage: 'Sound Volume' })}
                        disabled={soundVolume.disabled}
                        onKeyDownCapture={handleSliderParentKeyDownCapture}
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
                        onKeyDownCapture={handleSliderParentKeyDownCapture}
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

                <FieldSet title={intl.formatMessage({ defaultMessage: 'Boot Sound' })}>
                    <Field
                        title={intl.formatMessage({ defaultMessage: 'Enable Boot Sound' })}
                        description={intl.formatMessage({
                            defaultMessage:
                                'Enable sound during splash screen animation played at every startup of the Deck.',
                        })}
                        disabled={bootSoundEnabled.disabled}
                    >
                        <CarbonFormField error={bootSoundEnabled.error}>
                            <Toggle
                                id={$('sound', 'boot', 'enabled')}
                                size="md"
                                aria-invalid={!!bootSoundEnabled.error}
                                toggled={!!bootSoundEnabled.value}
                                onToggle={bootSoundEnabled.onChange}
                                disabled={bootSoundEnabled.disabled}
                            />
                        </CarbonFormField>
                    </Field>
                </FieldSet>

                <FieldSet title={intl.formatMessage({ defaultMessage: 'LED Notification Lights' })}>
                    <Field
                        title={intl.formatMessage({ defaultMessage: 'Enable LED Notifications' })}
                        description={intl.formatMessage({
                            defaultMessage: 'Use LED lights for notifications and alerts.',
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

                    <Field
                        title={intl.formatMessage({ defaultMessage: 'Enable LED Notifications in Night Mode' })}
                        description={intl.formatMessage({
                            defaultMessage: 'Use LED lights for notifications and alerts during Night Mode.',
                        })}
                        disabled={ledNotifyEnabled.disabled}
                    >
                        <CarbonFormField error={ledNotifyEnabledNight.error}>
                            <Toggle
                                id={$('led', 'notify-night', 'enabled')}
                                size="md"
                                aria-invalid={!!ledNotifyEnabledNight.error}
                                toggled={!!ledNotifyEnabledNight.value}
                                onToggle={ledNotifyEnabledNight.onChange}
                                disabled={ledNotifyEnabledNight.disabled}
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
