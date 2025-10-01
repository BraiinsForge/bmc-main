import { Component, Fragment } from 'react';
import { useIntl, type IntlShape } from 'react-intl';
import { getID, Form, type iField } from '@/lib/form';
import { toast } from '@/lib/toast';

// App
import * as pb from '@/proto';
import AppContext, { type AppContextType } from '@/context';

// Components
import { SoundOption } from '../SoundOption';
import { SoundPlayIcon } from '../SoundPlayIcon';
import { Button, CarbonFormField } from '@/components';
import { TextInput, Toggle, Dropdown } from '@carbon/react';

// CSS
import css from './AlarmForm.scss';

export interface AlarmFormProps {
    time: iField<string>; // hh:mm
    name: iField<string>;
    repeat: iField<pb.Weekday[]>;
    sound: iField<null | pb.SoundInfo['id']> & { options: pb.SoundInfo[] };

    snoozeEnabled: iField<boolean>;
    snoozeLimit: iField<pb.SnoozeLimit>;
    snoozeDuration: iField<pb.SnoozeDuration>;
}
export interface Props extends AlarmFormProps {
    intl: IntlShape;
}

interface State {
    isPlaying: boolean;
}

const $ = getID('alarm-form').get;
class View extends Component<Props, State> {
    static contextType = AppContext;
    declare context: AppContextType;

    constructor(props: Props, context: AppContextType) {
        super(props);
        const { currentlyPlaying } = context.device.sound;
        this.state = { isPlaying: currentlyPlaying?.id === props.sound.value };
    }
    componentDidUpdate(prevProps: Props) {
        const { sound } = this.props;
        const { isPlaying } = this.state;
        const { currentlyPlaying } = this.context.device.sound;

        // Something outside played a sound, but we are not marked as playing
        if (currentlyPlaying?.id === sound.value && !isPlaying) this.setState({ isPlaying: true });
        // We think we are playing a sound, but context says otherwise
        else if (isPlaying && !currentlyPlaying) this.setState({ isPlaying: false });
        // Sound changed and we are playing → switch
        else if (sound.value !== prevProps.sound.value && isPlaying) this.#playSelectedSound();
    }
    componentWillUnmount() {
        pb.abort.all(this);
    }

    #toggleDay = (day: pb.Weekday) => {
        const { value, onChange } = this.props.repeat;

        const val = value ?? [];
        const res = val.includes(day) ? val.filter(x => x !== day) : [...val, day];
        onChange(res.sort());
    };

    private abortPlaying = pb.abort.get();
    #playSelectedSound = async (): Promise<void> => {
        const { device } = this.context;
        const { sound, intl } = this.props;

        // In both play and stop case, we need to
        // first abort previous play request.
        // device.sound.stop();
        const { signal } = this.abortPlaying.replace();

        // If we don't have a sound selected,
        // there is nothing to do from here on.
        const selectedSoundObject = sound.options.find(x => x.id === sound.value);
        if (!selectedSoundObject) return;

        // Set playing, play, set not playing
        // (request is held as long as the sound is playing)
        this.setState({ isPlaying: true });

        try {
            await device.sound.play(selectedSoundObject, signal);
        } catch ($) {
            let msg = pb.collectAllErrorsAsFormattedList($);
            msg ||= intl.formatMessage({ defaultMessage: 'Failed to play the sound!' });

            toast.show('error', msg);
        }

        this.setState({ isPlaying: false });
    };
    #stopPlaying = () => this.abortPlaying.replace();

    #alarmSoundChange = (x: { selectedItem: null | pb.SoundInfo }): void => {
        const { sound } = this.props;
        sound.onChange(x.selectedItem?.id ?? null);
    };
    #alarmSoundToString = (value: null | pb.SoundInfo): string => value?.name ?? '--';
    #alarmSoundToElement = (value: pb.SoundInfo) => <SoundOption sound={value} />;

    render() {
        const { time, name, repeat, sound, snoozeEnabled, snoozeLimit, snoozeDuration, intl } = this.props;
        const { isPlaying } = this.state;
        const { formatMessage } = intl;

        return (
            <Form className={css.root}>
                <div className={css.rowTop}>
                    <div className={css.time}>
                        <TextInput
                            id={$('time')}
                            labelText={formatMessage({ defaultMessage: 'Time' })}
                            value={time.value ?? ''}
                            onChange={e => time.onChange(e.target.value)}
                            invalid={!!time.error}
                            invalidText={time.error}
                            placeholder={formatMessage({ defaultMessage: 'HH:MM' })}
                        />
                    </div>

                    <div className={css.name}>
                        <TextInput
                            id={$('name')}
                            labelText={formatMessage({ defaultMessage: 'Alarm Name (optional)' })}
                            value={name.value ?? ''}
                            onChange={e => name.onChange(e.target.value)}
                            invalid={!!name.error}
                            invalidText={name.error}
                            placeholder="---"
                        />
                    </div>
                </div>

                <CarbonFormField className={css.fullWidth} labelText={formatMessage({ defaultMessage: 'Repeat' })}>
                    <div
                        className={css.rowDays}
                        children={pb.weekdayOptionsAll.map(x => {
                            const key = pb.Weekday[x];
                            const label = pb.weekdayToString(intl, x);
                            const active: boolean = repeat.value?.includes(x) ?? false;

                            return (
                                <Button
                                    key={key}
                                    id={$('day-toggle', key)}
                                    size="sm"
                                    kind={active ? 'primary' : 'secondary'}
                                    children={label}
                                    onClick={() => this.#toggleDay(x)}
                                    blurOnClick={false}
                                />
                            );
                        })}
                    />
                </CarbonFormField>

                <CarbonFormField className={css.fullWidth} labelText={formatMessage({ defaultMessage: 'Alarm Sound' })}>
                    <div className={css.soundRow}>
                        <Dropdown<pb.SoundInfo>
                            id={$('sound')}
                            className={css.soundDropdown}
                            titleText=""
                            label={formatMessage({ defaultMessage: 'Select a sound…' })}
                            items={sound.options}
                            selectedItem={sound.options.find(x => x.id === sound.value)}
                            onChange={this.#alarmSoundChange}
                            itemToString={this.#alarmSoundToString}
                            itemToElement={this.#alarmSoundToElement}
                        />
                        <Button
                            id={$('play-selected-sound')}
                            kind="secondary"
                            size="md"
                            children={<SoundPlayIcon isPlaying={isPlaying} />}
                            onClick={isPlaying ? this.#stopPlaying : this.#playSelectedSound}
                            disabled={!sound.value}
                        />
                    </div>
                </CarbonFormField>

                <Toggle
                    id={$('snooze-enabled')}
                    toggled={!!snoozeEnabled.value}
                    labelText={formatMessage({ defaultMessage: 'Snooze' })}
                    labelA={formatMessage({ defaultMessage: 'Off' })}
                    labelB={formatMessage({ defaultMessage: 'On' })}
                    onToggle={snoozeEnabled.onChange}
                />

                {snoozeEnabled.value ? (
                    <Fragment>
                        <Dropdown<pb.SnoozeLimit>
                            id={$('snooze-limit')}
                            className={css.dropdown}
                            titleText={formatMessage({ defaultMessage: 'Snooze Limit' })}
                            label={formatMessage({ defaultMessage: 'Select snooze limit…' })}
                            helperText={formatMessage({
                                defaultMessage: 'Times alarm can be snoozed before dismissal',
                            })}
                            items={pb.alarmSnoozeLimitOptions}
                            selectedItem={snoozeLimit.value ?? undefined}
                            onChange={x => {
                                const v = x.selectedItem;
                                if (v != null) snoozeLimit.onChange(v);
                            }}
                            itemToString={x => pb.alarmSnoozeLimitToString(intl, x) ?? 'N/A'}
                        />

                        <Dropdown<pb.SnoozeDuration>
                            id={$('snooze-duration')}
                            className={css.dropdown}
                            titleText={formatMessage({ defaultMessage: 'Snooze Duration' })}
                            label={formatMessage({ defaultMessage: 'Select snooze duration…' })}
                            items={pb.alarmSnoozeDurationOptions}
                            selectedItem={snoozeDuration.value ?? undefined}
                            onChange={x => {
                                const v = x.selectedItem;
                                if (v !== null) snoozeDuration.onChange(v);
                            }}
                            itemToString={x => pb.alarmSnoozeDurationToString(intl, x) ?? 'N/A'}
                        />
                    </Fragment>
                ) : null}
            </Form>
        );
    }
}
export function AlarmForm(props: AlarmFormProps) {
    const intl = useIntl();
    return <View {...props} intl={intl} />;
}
