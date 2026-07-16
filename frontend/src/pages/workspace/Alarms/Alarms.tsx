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
import { debounce } from 'es-toolkit';
import { Helmet } from '@dr.pogodin/react-helmet';
import { useIntl, type IntlShape, FormattedMessage } from 'react-intl';

import { getID } from '@/lib/form';
import { setState } from '@/lib/react';
import { toast } from '@/lib/toast';
import { formatAlarmTime, parseAlarmTime, validateTime } from '@/lib/time';

// App
import * as pb from '@/proto';
import AppContext, { type AppContextType } from '@/context';

// Components
import { Add as IconAdd, TrashCan as IconDelete } from '@carbon/react/icons';
import { AlarmsTable, AlarmForm } from './components';
import { Button, Modal, InlineNotification } from '@/components';

// Styles
import css from './Alarms.scss';

interface Props {
    intl: IntlShape;
}

type FormValues = Pick<pb.AddAlarmRequest, 'time' | 'name' | 'soundId' | 'repeat'> & {
    snoozeEnabled: boolean;
    snoozeLimit: null | pb.SnoozeLimit;
    snoozeDuration: null | pb.SnoozeDuration;
};
type FormState = pb.FormState<FormValues>;

interface State {
    isLoading: boolean;

    alarms: pb.Alarm[];
    sounds: pb.SoundInfo[];
    alarmDefaults: pb.AlarmInfoResponse;
    timeFormat: pb.TimeFormat;

    openDialog: null | {
        key: 'alarm';
        editingID: null | pb.Alarm['id'];
        data: FormState;
    };
}
const getInitialState = (): State => ({
    isLoading: false,

    alarms: [],
    sounds: [],
    alarmDefaults: pb.create(pb.AlarmInfoResponseSchema),
    timeFormat: pb.TimeFormat.TIME_FORMAT_24_HOUR,

    openDialog: null,
});

const $ = getID('alarms').get;

export class View extends Component<Props, State> {
    static contextType = AppContext;
    declare context: AppContextType;

    readonly state = getInitialState();

    componentDidMount() {
        this.#load();
    }
    componentWillUnmount() {
        pb.abort.all(this);
    }

    private abortLoad = pb.abort.get();
    #load = async () => {
        const { formatMessage } = this.props.intl;

        const { signal } = this.abortLoad.replace();
        const reqOpts = { signal };
        await setState(this, { isLoading: true });

        try {
            const [{ alarms }, { sounds }, alarmDefaults, generalSettings] = await Promise.all([
                pb.rpc.alarm.listAlarms({}, reqOpts),
                pb.rpc.config.listSounds({}, reqOpts),
                pb.rpc.alarm.getAlarmInfo({}, reqOpts),
                pb.rpc.config.getGeneralSettingsData({}, reqOpts),
            ]);
            const timeFormat = generalSettings.timeFormat || pb.TimeFormat.TIME_FORMAT_24_HOUR;
            this.setState({ isLoading: false, alarms, sounds, alarmDefaults, timeFormat });
        } catch ($) {
            if (pb.abort.is($)) return;

            let msg = pb.collectAllErrorsAsFormattedList($);
            msg ||= formatMessage({ defaultMessage: 'Failed to load data for alarms!' });

            toast.error(msg);
        } finally {
            this.setState({ isLoading: false });
        }
    };
    #loadDebounced = debounce(this.#load, 250);

    get #txt() {
        const { formatMessage } = this.props.intl;
        return {
            title: formatMessage({ defaultMessage: 'Alarms' }),
            blurb: formatMessage({
                defaultMessage: 'Let’s make sure your alarm wakes you just right – not too loud, not too chill.',
            }),
            addNewAlarm: formatMessage({ defaultMessage: 'Add New Alarm' }),
            editAlarm: formatMessage({ defaultMessage: 'Edit Alarm' }),
            confirmChanges: formatMessage({ defaultMessage: 'Confirm Changes' }),
            cancel: formatMessage({ defaultMessage: 'Cancel' }),
            deleteAlarm: formatMessage({ defaultMessage: 'Delete Alarm' }),
        };
    }

    //
    // Add dialog
    //

    #alarmDialogOpen = (): void => {
        this.setState(s => {
            const d = s.alarmDefaults;
            const snooze = d.snoozeOptions;

            return {
                ...s,
                openDialog: {
                    key: 'alarm',
                    editingID: null,
                    data: {
                        values: {
                            time: formatAlarmTime(d.time, s.timeFormat),
                            name: d.name,
                            repeat: d.repeat,
                            soundId: d.soundId,

                            snoozeEnabled: snooze?.kind.case === 'snooze',
                            snoozeLimit: snooze?.kind.case === 'snooze' ? snooze?.kind.value.limit : null,
                            snoozeDuration: snooze?.kind.case === 'snooze' ? snooze?.kind.value.duration : null,
                        } satisfies FormValues,
                        errors: null,
                    },
                },
            } satisfies State;
        });
    };
    #alarmDialogClose = () => this.setState({ openDialog: null });

    private abortAddSubmit = pb.abort.get();
    #alarmDialogSubmit = async (): Promise<void> => {
        const { formatMessage } = this.props.intl;
        const { openDialog, alarms } = this.state;

        if (openDialog?.key !== 'alarm') {
            toast.error(
                formatMessage({ defaultMessage: "Invalid state, can't create alarm when create dialog is not open!" }),
            );
            return;
        }

        const { editingID } = openDialog;
        const {
            time: rawTime,
            name,
            repeat,
            soundId,
            snoozeEnabled,
            snoozeDuration,
            snoozeLimit,
        } = openDialog.data.values;

        const trimmedTime = rawTime.trim();

        if (!validateTime(trimmedTime, this.state.timeFormat)) {
            this.setState(s => {
                if (s.openDialog?.key !== 'alarm') return s;
                return {
                    ...s,
                    openDialog: {
                        ...s.openDialog,
                        data: {
                            ...s.openDialog.data,
                            errors: {
                                global: [],
                                fields: { time: [formatMessage({ defaultMessage: 'Invalid time format' })] },
                            },
                        },
                    },
                };
            });
            return;
        }

        const time = parseAlarmTime(trimmedTime, this.state.timeFormat);
        const snoozeOptions = pb.create(pb.SnoozeOptionsWrapperSchema, {
            kind: snoozeEnabled
                ? {
                      case: 'snooze',
                      value: {
                          $typeName: 'braiins.bmc.web.SnoozeOptions',
                          limit: snoozeLimit ?? pb.SnoozeLimit.SNOOZE_LIMIT_FOREVER,
                          duration: snoozeDuration ?? pb.SnoozeDuration.SNOOZE_DURATION_5_MINUTES,
                      },
                  }
                : {
                      case: 'off',
                      value: { $typeName: 'braiins.bmc.web.Off' },
                  },
        });
        const enabled: boolean = alarms.find(x => x.id === editingID)?.enabled ?? false;

        try {
            const { signal } = this.abortAddSubmit.replace();

            // Edit
            if (editingID?.length) {
                await pb.rpc.alarm.setAlarm(
                    pb.create(pb.SetAlarmRequestSchema, {
                        id: editingID,
                        name,
                        time,
                        repeat,
                        soundId,
                        enabled,
                        snoozeOptions,
                    }),
                    { signal },
                );
                toast.success(formatMessage({ defaultMessage: 'Alarm has been updated' }));
            }

            // Create
            else {
                await pb.rpc.alarm.addAlarm(
                    pb.create(pb.AddAlarmRequestSchema, {
                        enabled: true,
                        time,
                        name,
                        repeat,
                        soundId,
                        snoozeOptions,
                    }),
                    { signal },
                );
                toast.success(formatMessage({ defaultMessage: 'Alarm has been added' }));
            }

            this.#alarmDialogClose();
        } catch ($) {
            if (pb.abort.is($)) return;
            const formErrors = pb.parseFormErrors<pb.AddAlarmRequest>($, [
                'name',
                'time',
                'enabled',
                'repeat',
                'soundId',
                'snoozeOptions',
            ]);
            this.setState(s => {
                const { openDialog } = s;
                if (openDialog?.key !== 'alarm') return s;

                return {
                    ...s,
                    openDialog: {
                        ...openDialog,
                        data: {
                            ...openDialog.data,
                            errors: formErrors,
                        },
                    },
                };
            });
        } finally {
            this.#loadDebounced();
        }
    };

    #alarmDialogGetFieldValue = <Key extends keyof FormValues>(key: Key) => {
        const { openDialog } = this.state;
        if (openDialog?.key !== 'alarm') return null;
        return openDialog.data.values?.[key] ?? null;
    };
    #alarmDialogGetFieldError = <Key extends keyof FormValues>(key: Key) => {
        const { openDialog } = this.state;
        if (openDialog?.key !== 'alarm') return null;

        const e = openDialog.data.errors?.fields?.[key];
        // FIXME: Handle special case of the "repeat" field where our type util does not work well
        const errors = Array.isArray(e) && e.every(x => typeof x === 'string') ? e : null;
        return pb.renderFieldErrorsAsList(errors);
    };
    #alarmDialogGetChangeHandler = <Key extends keyof FormValues>(key: Key) => {
        return (value: null | FormValues[Key]): void => {
            this.setState(s => {
                const { openDialog } = s;
                if (openDialog?.key !== 'alarm') return s;

                return {
                    ...s,
                    openDialog: {
                        ...openDialog,
                        data: {
                            values: {
                                ...openDialog.data.values,
                                [key]: value,
                            },
                            errors: null,
                        },
                    },
                };
            });
        };
    };
    #alarmDialogGetFieldStruct = <Key extends keyof FormValues>(key: Key) => {
        return {
            value: this.#alarmDialogGetFieldValue(key),
            error: this.#alarmDialogGetFieldError(key),
            onChange: this.#alarmDialogGetChangeHandler(key),
        };
    };

    #addRender = (): ReactElement => {
        const { formatMessage } = this.props.intl;
        const { openDialog, sounds, timeFormat } = this.state;
        const data = openDialog?.key === 'alarm' ? openDialog.data : null;

        const txt = this.#txt;

        const alarmID = openDialog?.editingID;
        const isEdit = alarmID != null;
        const labelTitle = isEdit ? txt.editAlarm : txt.addNewAlarm;
        const labelSubmit = isEdit ? txt.confirmChanges : txt.addNewAlarm;

        return (
            <Modal
                id={$('add-dialog')}
                open={!!data}
                size="sm"
                // Labeling & behavior
                selectorPrimaryFocus="input"
                modalHeading={labelTitle}
                // Submit
                primaryButtonText={labelSubmit}
                onRequestSubmit={this.#alarmDialogSubmit}
                // Cancel
                onRequestClose={this.#alarmDialogClose}
                secondaryButtonText={txt.cancel}
                onSecondarySubmit={this.#alarmDialogClose}
            >
                {data?.errors?.global ? (
                    <InlineNotification
                        kind="error"
                        stretch
                        hideCloseButton
                        children={pb.renderFieldErrorsAsList(data.errors.global)}
                        style={{ marginBottom: '1rem' }}
                    />
                ) : null}

                <AlarmForm
                    time={this.#alarmDialogGetFieldStruct('time')}
                    timeFormat={timeFormat}
                    name={this.#alarmDialogGetFieldStruct('name')}
                    repeat={this.#alarmDialogGetFieldStruct('repeat')}
                    sound={{ ...this.#alarmDialogGetFieldStruct('soundId'), options: sounds }}
                    // Snooze
                    snoozeEnabled={this.#alarmDialogGetFieldStruct('snoozeEnabled')}
                    snoozeLimit={this.#alarmDialogGetFieldStruct('snoozeLimit')}
                    snoozeDuration={this.#alarmDialogGetFieldStruct('snoozeDuration')}
                />

                {alarmID != null ? (
                    <div className={css.deleteButtonRow}>
                        <Button
                            id={$('delete-alarm', alarmID)}
                            kind="danger"
                            icon={IconDelete}
                            children={formatMessage({ defaultMessage: 'Delete Alarm' })}
                            onClick={() => this.#onDelete(alarmID)}
                        />
                    </div>
                ) : null}
            </Modal>
        );
    };

    //
    // /Add dialog
    //

    #onEdit = (id: pb.Alarm['id']): void => {
        this.setState(s => {
            const d = s.alarms.find(x => x.id === id);
            if (!d) return s;

            const snooze = d.snoozeOptions;

            return {
                ...s,
                openDialog: {
                    key: 'alarm',
                    editingID: id,
                    data: {
                        values: {
                            time: formatAlarmTime(d.time, s.timeFormat),
                            name: d.name,
                            repeat: d.repeat,
                            soundId: d.sound?.id,

                            snoozeEnabled: snooze?.kind.case === 'snooze',
                            snoozeLimit: snooze?.kind.case === 'snooze' ? snooze?.kind.value.limit : null,
                            snoozeDuration: snooze?.kind.case === 'snooze' ? snooze?.kind.value.duration : null,
                        } satisfies FormValues,
                        errors: null,
                    } satisfies FormState,
                },
            } satisfies State;
        });
    };
    #onToggle = async (id: pb.Alarm['id'], enabled: boolean): Promise<void> => {
        const { formatMessage } = this.props.intl;

        try {
            // First a positive update
            this.setState(s => ({ alarms: s.alarms.map(x => (x.id === id ? { ...x, enabled } : x)) }));

            // Then submit to API
            await pb.rpc.alarm.setAlarmEnabled({ id, enabled });
            toast.success(formatMessage({ defaultMessage: 'Alarm has been successfully toggled.' }));
        } catch ($) {
            if (pb.abort.is($)) return;
            toast.error(formatMessage({ defaultMessage: 'Failed to toggle alarm!' }));
        } finally {
            // Always reload data to make sure
            // we have the latest state
            this.#loadDebounced();
        }
    };
    #onDelete = async (id: pb.Alarm['id']): Promise<void> => {
        const { confirm } = this.context;
        const { intl } = this.props;
        const { alarms, timeFormat } = this.state;

        const d = alarms.find(x => x.id === id);
        if (!d) return;

        const confirmed = await confirm({
            size: 'xs',
            danger: true,
            title: intl.formatMessage({ defaultMessage: 'Delete Alarm' }),
            confirmLabel: intl.formatMessage({ defaultMessage: 'Delete' }),
            message: (
                <div className={css.deleteConfirmationMessage}>
                    <FormattedMessage defaultMessage="Are you sure you want to delete this alarm?" />

                    <table>
                        <tbody>
                            <tr>
                                <FormattedMessage tagName="th" defaultMessage="Time" />
                                <td children={formatAlarmTime(d.time, timeFormat)} />
                            </tr>
                            <tr>
                                <FormattedMessage tagName="th" defaultMessage="Label" />
                                <td children={d.name || '--'} />
                            </tr>
                            <tr>
                                <FormattedMessage tagName="th" defaultMessage="Repeat" />
                                <td children={pb.weekdayListToString(intl, d.repeat)} />
                            </tr>
                            <tr>
                                <FormattedMessage tagName="th" defaultMessage="Sound" />
                                <td children={d.sound?.name || '--'} />
                            </tr>
                            <tr>
                                <FormattedMessage tagName="th" defaultMessage="Snooze" />
                                <td children={pb.alarmSnoozeOptionsToString(intl, d.snoozeOptions)} />
                            </tr>
                        </tbody>
                    </table>
                </div>
            ),
        });
        if (!confirmed) return;

        try {
            // Positive update first
            this.setState(s => ({ alarms: s.alarms.filter(x => x.id !== id) }));

            // Then submit to API
            await pb.rpc.alarm.deleteAlarm({ value: id });
            this.#alarmDialogClose();
            toast.success(intl.formatMessage({ defaultMessage: 'Alarm has been deleted.' }));
        } catch ($) {
            if (pb.abort.is($)) return;
            let msg = pb.collectAllErrorsAsFormattedList($);
            msg ||= intl.formatMessage({ defaultMessage: 'Failed to delete alarm!' });
            toast.error(msg);
        } finally {
            // Always reload data to make sure
            // we have the latest state
            this.#loadDebounced();
        }
    };

    render() {
        const { /* isLoading, */ alarms, timeFormat } = this.state;
        const txt = this.#txt;

        return (
            <div className={css.root}>
                <Helmet title={txt.title} />

                <div className={css.header}>
                    <div className={css.labels}>
                        <h1 className={css.title} children={txt.title} />
                        <div className={css.description} children={txt.blurb} />
                    </div>

                    <div className={css.actions}>
                        <Button
                            id={$('add-alarm')}
                            renderIcon={IconAdd}
                            children={txt.addNewAlarm}
                            onClick={this.#alarmDialogOpen}
                        />
                    </div>
                </div>

                <div className={css.content}>
                    <AlarmsTable
                        isLoading={false}
                        data={alarms}
                        timeFormat={timeFormat}
                        onEdit={this.#onEdit}
                        onToggle={this.#onToggle}
                        onDelete={this.#onDelete}
                    />
                </div>

                {this.#addRender()}
            </div>
        );
    }
}

export default function AlarmsPage() {
    const intl = useIntl();
    return <View intl={intl} />;
}
