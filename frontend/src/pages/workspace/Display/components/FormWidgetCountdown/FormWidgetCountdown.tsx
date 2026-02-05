import { Component } from 'react';
import { useIntl, type IntlShape } from 'react-intl';
import { format, parse } from 'date-fns';

import * as pb from '@/proto';
import { getID } from '../const';
import { Form, type iField, type FormPropsToValuesRec } from '@/lib/form';

// Components
import {
    WidgetSizeSelector,
    type WidgetSizeSelectorProps,
    BoundRadioGroup,
    BoundCheckbox,
    type OptionItem,
    CheckYourScreenForPreview,
} from '../shared';
import { ModalCustom, InlineNotification, Button, ColorInput } from '@/components';
import { CheckboxGroup, TextInput, NumberInput, Dropdown } from '@carbon/react';

// styles
import css from '../shared.scss';

const $ = getID('countdown-form').get;

function rgbToHex(r: number, g: number, b: number): string {
    return `#${[r, g, b].map(v => v.toString(16).padStart(2, '0')).join('')}`;
}

export interface FormWidgetCountdownProps {
    isOpen: boolean;
    isEdit: boolean;
    onClose(): void;
    error: Maybe<string>;

    widgetSize: WidgetSizeSelectorProps['field'];

    label: iField<string>;
    targetDate: iField<string>; // ISO date string (YYYY-MM-DD)
    targetTime: iField<string>; // Time string (HH:mm)
    backgroundColor: iField<string>;
    fontStyle: iField<pb.FontStyle>;

    // Completion action settings
    ledEnabled: iField<boolean>;
    ledEffect: iField<pb.LedEffect>;
    ledColorR: iField<number>;
    ledColorG: iField<number>;
    ledColorB: iField<number>;
    soundEnabled: iField<boolean>;
    soundId: iField<string>;
    soundVolume: iField<number>;

    soundOptions: pb.SoundInfo[];

    style?: CSSProperties;
}

interface Props extends FormWidgetCountdownProps {
    intl: IntlShape;
}

class View extends Component<Props> {
    get #fontStyleOptions(): Array<OptionItem<pb.FontStyle>> {
        const { formatMessage } = this.props.intl;
        return [
            { value: pb.FontStyle.LIGHT, label: formatMessage({ defaultMessage: 'Light' }) },
            { value: pb.FontStyle.MEDIUM, label: formatMessage({ defaultMessage: 'Medium' }) },
            { value: pb.FontStyle.BOLD, label: formatMessage({ defaultMessage: 'Bold' }) },
        ];
    }

    get #ledEffectOptions(): Array<OptionItem<pb.LedEffect>> {
        const { formatMessage } = this.props.intl;
        return [
            { value: pb.LedEffect.NONE, label: formatMessage({ defaultMessage: 'None' }) },
            { value: pb.LedEffect.SOLID, label: formatMessage({ defaultMessage: 'Solid' }) },
            { value: pb.LedEffect.BREATHE, label: formatMessage({ defaultMessage: 'Breathe' }) },
            { value: pb.LedEffect.CHASE, label: formatMessage({ defaultMessage: 'Chase' }) },
            { value: pb.LedEffect.KNIGHT_RIDER, label: formatMessage({ defaultMessage: 'Knight Rider' }) },
            { value: pb.LedEffect.SCAN, label: formatMessage({ defaultMessage: 'Scan' }) },
            { value: pb.LedEffect.SNAKE, label: formatMessage({ defaultMessage: 'Snake' }) },
        ];
    }

    #soundChange = (x: { selectedItem: null | pb.SoundInfo }): void => {
        const { soundId } = this.props;
        soundId.onChange?.(x.selectedItem?.id ?? '');
    };
    #soundToString = (value: null | pb.SoundInfo): string => value?.name ?? '--';

    #txt = {
        countdown: this.props.intl.formatMessage({ defaultMessage: 'Countdown' }),
        addWidget: this.props.intl.formatMessage({ defaultMessage: 'Add Widget' }),
        editWidget: this.props.intl.formatMessage({ defaultMessage: 'Edit Widget' }),
    };

    render() {
        const {
            isOpen,
            isEdit,
            onClose,
            error,

            // Main
            widgetSize,
            label,
            targetDate,
            targetTime,
            backgroundColor,
            fontStyle,

            // Completion actions
            ledEnabled,
            ledEffect,
            ledColorR,
            ledColorG,
            ledColorB,
            soundEnabled,
            soundId,
            soundVolume,
            soundOptions,

            style,
            intl: { formatMessage },
        } = this.props;

        const form = (
            <Form className={css.form} style={style}>
                <WidgetSizeSelector field={widgetSize} />

                <TextInput
                    id={$('label')}
                    labelText={formatMessage({ defaultMessage: 'Label' })}
                    placeholder={formatMessage({ defaultMessage: 'My Countdown' })}
                    value={label.value ?? ''}
                    onChange={e => label.onChange?.(e.target.value)}
                    invalid={!!label.error}
                    invalidText={label.error}
                />

                <TextInput
                    id={$('target-date')}
                    type="date"
                    labelText={formatMessage({ defaultMessage: 'Target Date' })}
                    value={targetDate.value ?? ''}
                    onChange={e => targetDate.onChange?.(e.target.value)}
                    invalid={!!targetDate.error}
                    invalidText={targetDate.error}
                />

                <TextInput
                    id={$('target-time')}
                    type="time"
                    labelText={formatMessage({ defaultMessage: 'Target Time' })}
                    value={targetTime.value ?? ''}
                    onChange={e => targetTime.onChange?.(e.target.value)}
                    invalid={!!targetTime.error}
                    invalidText={targetTime.error}
                />

                <div className={css.colorInputs}>
                    <TextInput
                        id={$('background-color')}
                        labelText={formatMessage({ defaultMessage: 'Background Color (optional)' })}
                        placeholder={formatMessage({ defaultMessage: '#000000 or black' })}
                        value={backgroundColor.value ?? ''}
                        onChange={e => backgroundColor.onChange?.(e.target.value)}
                        helperText={formatMessage({ defaultMessage: 'Hex color (#RGB, #RRGGBB) or color name' })}
                    />
                    <ColorInput
                        id={$('background-color-picker')}
                        value={backgroundColor.value || '#000000'}
                        onChange={backgroundColor.onChange}
                        aria-label={formatMessage({ defaultMessage: 'Pick background color' })}
                        style={{ marginTop: 24 }}
                    />
                </div>

                <BoundRadioGroup
                    {...fontStyle}
                    id={$('font')}
                    labelText={formatMessage({ defaultMessage: 'Numbers Font Style' })}
                    items={this.#fontStyleOptions}
                />

                <CheckboxGroup legendText={formatMessage({ defaultMessage: 'Completion Actions' })}>
                    <BoundCheckbox
                        {...ledEnabled}
                        id={$('led-enabled')}
                        labelText={formatMessage({ defaultMessage: 'Enable LED Animation' })}
                    />
                    <BoundCheckbox
                        {...soundEnabled}
                        id={$('sound-enabled')}
                        labelText={formatMessage({ defaultMessage: 'Play Sound' })}
                    />
                </CheckboxGroup>

                {ledEnabled.value && (
                    <>
                        <BoundRadioGroup
                            {...ledEffect}
                            id={$('led-effect')}
                            labelText={formatMessage({ defaultMessage: 'LED Effect' })}
                            items={this.#ledEffectOptions}
                        />
                        <ColorInput
                            id={$('led-color')}
                            labelText={formatMessage({ defaultMessage: 'LED Color' })}
                            value={rgbToHex(ledColorR.value ?? 0, ledColorG.value ?? 0, ledColorB.value ?? 0)}
                            onChange={(_, { r, g, b }) => {
                                ledColorR.onChange?.(r);
                                ledColorG.onChange?.(g);
                                ledColorB.onChange?.(b);
                            }}
                        />
                    </>
                )}

                {soundEnabled.value && (
                    <>
                        <Dropdown<null | pb.SoundInfo>
                            id={$('sound-id')}
                            titleText={formatMessage({ defaultMessage: 'Sound' })}
                            label={formatMessage({ defaultMessage: 'Select a sound...' })}
                            items={soundOptions}
                            selectedItem={soundOptions.find(x => x.id === soundId.value) ?? null}
                            onChange={this.#soundChange}
                            itemToString={this.#soundToString}
                        />
                        <NumberInput
                            id={$('sound-volume')}
                            label={formatMessage({ defaultMessage: 'Volume (%)' })}
                            min={0}
                            max={100}
                            value={soundVolume.value ?? ''}
                            onChange={(_, { value }) => soundVolume.onChange?.(Number(value))}
                        />
                    </>
                )}

                <CheckYourScreenForPreview />

                {error ? (
                    <InlineNotification
                        kind="error"
                        theme="inverse"
                        stretch
                        hideCloseButton
                        title={formatMessage({ defaultMessage: 'Error' })}
                        children={error}
                    />
                ) : null}
            </Form>
        );

        const verb = isEdit ? this.#txt.editWidget : this.#txt.addWidget;

        return (
            <ModalCustom
                id={$('dialog')}
                className={css.modal}
                selectorPrimaryFocus="form input,button"
                // State
                size="sm"
                open={isOpen}
                // Heading
                title={this.#txt.countdown}
                label={verb}
                // Cancel
                onClose={onClose}
                // Content
                children={form}
                footer={
                    <Button
                        id={$('done')}
                        kind="primary"
                        children={formatMessage({ defaultMessage: 'Done' })}
                        onClick={onClose}
                    />
                }
            />
        );
    }
}

export function FormWidgetCountdown(props: FormWidgetCountdownProps) {
    const intl = useIntl();
    return <View {...props} intl={intl} />;
}

/**
 * Convert date and time strings to Unix timestamp (seconds)
 */
function dateTimeToTimestamp(date: Maybe<string>, time: Maybe<string>): bigint {
    if (!date || !time) return BigInt(Math.floor(Date.now() / 1_000) + 3_600); // Default: 1 hour from now

    const d = parse(`${date} ${time || '00:00'}`, 'yyyy-MM-dd HH:mm', new Date());
    return BigInt(Math.floor(d.getTime() / 1000));
}

/**
 * Convert Unix timestamp (seconds) to date and time strings in local timezone
 */
function timestampToDateTime(timestamp: bigint): { date: string; time: string } {
    const d = new Date(Number(timestamp) * 1000);
    return { date: format(d, 'yyyy-MM-dd'), time: format(d, 'HH:mm') };
}

export function createCountdownWidgetKind(data: FormPropsToValuesRec<FormWidgetCountdownProps>): pb.WidgetKind {
    const led: pb.LedSettings | undefined = data.ledEnabled
        ? pb.create(pb.LedSettingsSchema, {
              effect: data.ledEffect,
              color: pb.create(pb.RgbColorSchema, {
                  r: data.ledColorR,
                  g: data.ledColorG,
                  b: data.ledColorB,
              }),
          })
        : undefined;

    const sound: pb.SoundSettings | undefined = data.soundEnabled
        ? pb.create(pb.SoundSettingsSchema, {
              soundId: data.soundId || '',
              volume: data.soundVolume,
          })
        : undefined;

    const completionAction: pb.CountdownCompletionAction | undefined =
        led || sound ? pb.create(pb.CountdownCompletionActionSchema, { led, sound }) : undefined;

    const seconds = dateTimeToTimestamp(data.targetDate, data.targetTime);
    return pb.create(pb.WidgetKindSchema, {
        value: {
            case: 'countdown',
            value: pb.create(pb.CountdownWidgetSchema, {
                label: data.label,
                targetTimestamp: pb.create(pb.TimestampSchema, { seconds, nanos: 0 }),
                backgroundColor: data.backgroundColor || undefined,
                numbersFontStyle: data.fontStyle,
                completionAction,
            }),
        },
    });
}

export function unpackCountdownWidgetKind(
    data: pb.WidgetKind,
    widgetSize: pb.WidgetSize,
): FormPropsToValuesRec<FormWidgetCountdownProps> {
    if (data.value.case !== 'countdown') throw new Error('Invalid widget kind');
    const countdown = data.value.value;

    const { date, time } = timestampToDateTime(countdown.targetTimestamp?.seconds ?? 0n);
    const action = countdown.completionAction;
    const led = action?.led;
    const sound = action?.sound;

    return {
        widgetSize,
        label: countdown.label,
        targetDate: date,
        targetTime: time,
        backgroundColor: countdown.backgroundColor ?? '',
        fontStyle: countdown.numbersFontStyle,
        ledEnabled: led != null,
        ledEffect: led?.effect ?? pb.LedEffect.UNSPECIFIED,
        ledColorR: led?.color?.r ?? 0,
        ledColorG: led?.color?.g ?? 0,
        ledColorB: led?.color?.b ?? 0,
        soundEnabled: sound != null,
        soundId: sound?.soundId ?? '',
        soundVolume: sound?.volume ?? 0,
    };
}
