import { Component } from 'react';
import { useIntl, type IntlShape } from 'react-intl';
import { useNavigate, type NavigateFunction } from 'react-router';

// App, lib
import * as pb from '@/proto';
import { URLS } from '@/constants';
import { setState } from '@/lib/react';
import type { FormPropsToLocalState } from '@/lib/form';

// Components
import { InlineNotificationsGroup } from '@/components';
import { Setup, type SetupProps } from './components';

// Styles
import '@/styles/carbon/carbon.global.scss';
import css from './Init.scss';

interface Props {
    intl: IntlShape;
    navigate: NavigateFunction;
}

type FormState = FormPropsToLocalState<SetupProps>;
interface State {
    isLoading: boolean;
    isSaving: boolean;

    data: FormState;
    timezones: Array<pb.Timezone>;
}
const getInitialState = (): State => ({
    isLoading: false,
    isSaving: false,

    data: {
        values: {},
        errors: null,
    },
    timezones: [],
});

class View extends Component<Props, State> {
    readonly state = getInitialState();

    componentDidMount = () => this.#loadConfig();
    componentWillUnmount = () => pb.abort.all(this);

    private abortLoadConfig = pb.abort.get();
    #loadConfig = async (): Promise<void> => {
        const { formatMessage, timeZone } = this.props.intl;
        const { signal } = this.abortLoadConfig.replace();

        await setState(this, { isLoading: true });
        let timezones: Array<pb.Timezone> = [];
        const res: FormState = {
            values: {},
            errors: null,
        };

        try {
            const v = await pb.rpc.init.getSettingsData({}, { signal });
            timezones = v.timezones;

            // Try to detect browser timezone from react-intl first, fallback to Intl API, then server default
            const browserTimezone = timeZone ?? Intl.DateTimeFormat().resolvedOptions().timeZone;
            const selectedTimezone =
                timezones.find(x => x.id === browserTimezone) ?? timezones.find(x => x.id === v.timezoneId);

            res.values = {
                timezone: selectedTimezone,
                timeFormat: v.timeFormat || undefined,
                dateFormat: v.dateFormat || undefined,
                numberFormat: v.numberFormat || undefined,
                temperatureUnits: v.temperatureUnit || undefined,
                unitSystem: v.unitSystem || undefined,
                // dataCollection: v.dataCollection,
            };
        } catch ($) {
            if (pb.abort.is($)) return;
            const errors: string[] = pb.collectAllErrors($) ?? [
                formatMessage({ defaultMessage: 'Failed to load initial values!' }),
            ];
            this.setState(s => ({
                data: {
                    ...s.data,
                    errors: { global: errors },
                },
            }));
        }

        this.setState({ isLoading: false, timezones, data: res });
    };

    #handleChange = <Key extends keyof FormState['values']>(key: Key) => {
        return (value: FormState['values'][Key]): void => {
            this.setState(s => ({
                data: {
                    errors: null,
                    values: {
                        ...s.data.values,
                        [key]: value,
                    },
                },
            }));
        };
    };
    #getFieldError = <Key extends keyof FormState['values']>(key: Key): Maybe<string> => {
        const { errors } = this.state.data;
        return pb.renderFieldErrorsAsList(errors?.fields?.[key]);
    };

    private abortSubmit = pb.abort.get();
    #submit = async (): Promise<void> => {
        const {
            // Security
            password1,
            password2,

            // Privacy
            // dataCollection,

            // Time & format
            timezone,
            dateFormat,
            numberFormat,
            timeFormat,
            temperatureUnits,
            unitSystem,
        } = this.state.data.values;
        const {
            intl: { formatMessage },
            navigate,
        } = this.props;

        if (password1 != null && password1 !== password2) {
            this.setState(s => ({
                data: {
                    ...s.data,
                    errors: {
                        fields: {
                            password2: [formatMessage({ defaultMessage: 'Passwords have to match!' })],
                        },
                    },
                },
            }));
            return;
        }

        const { signal } = this.abortSubmit.replace();
        try {
            await pb.rpc.init.setupDevice(
                pb.create(pb.SettingsRequestSchema, {
                    // dataCollection,
                    dateFormat,
                    numberFormat,
                    password: password1,
                    timezoneId: timezone?.id,
                    timeFormat,
                    temperatureUnit: temperatureUnits,
                    unitSystem,
                }),
                { signal },
            );
            navigate(URLS.auth.login, { replace: true });
        } catch ($) {
            if (pb.abort.is($)) return;
            const errors = pb.parseFormErrors($, [
                'password1',
                'password2',
                'timezone',
                'dateFormat',
                'numberFormat',
                'timeFormat',
                'temperatureUnits',
                'unitSystem',
                // 'dataCollection',
            ]);
            this.setState(s => ({ data: { ...s.data, errors } }));
        }
    };

    render() {
        const {
            isLoading,
            isSaving,
            timezones,
            data: { values, errors },
        } = this.state;

        const disabled: boolean = isLoading || isSaving;

        return (
            <div className={css.root}>
                <div className={css.innerSetup}>
                    <InlineNotificationsGroup kind="error" theme="inverse" items={errors?.global} stretch />
                    <Setup
                        timeFormat={{
                            disabled,
                            value: values.timeFormat || null,
                            error: this.#getFieldError('timeFormat'),
                            onChange: this.#handleChange('timeFormat'),
                        }}
                        timezone={{
                            value: values.timezone || null,
                            disabled,
                            items: timezones,
                            error: this.#getFieldError('timezone'),
                            onChange: this.#handleChange('timezone'),
                        }}
                        dateFormat={{
                            disabled,
                            value: values.dateFormat || null,
                            error: this.#getFieldError('dateFormat'),
                            onChange: this.#handleChange('dateFormat'),
                        }}
                        numberFormat={{
                            disabled,
                            value: values.numberFormat || null,
                            error: this.#getFieldError('numberFormat'),
                            onChange: this.#handleChange('numberFormat'),
                        }}
                        temperatureUnits={{
                            disabled,
                            value: values.temperatureUnits || null,
                            error: this.#getFieldError('temperatureUnits'),
                            onChange: this.#handleChange('temperatureUnits'),
                        }}
                        unitSystem={{
                            disabled,
                            value: values.unitSystem || null,
                            error: this.#getFieldError('unitSystem'),
                            onChange: this.#handleChange('unitSystem'),
                        }}
                        // Password
                        password1={{
                            disabled,
                            value: values.password1 || null,
                            error: this.#getFieldError('password1'),
                            onChange: this.#handleChange('password1'),
                        }}
                        password2={{
                            disabled,
                            value: values.password2 || null,
                            error: this.#getFieldError('password2'),
                            onChange: this.#handleChange('password2'),
                        }}
                        // // Privacy
                        // dataCollection={{
                        //     disabled,
                        //     value: values.dataCollection || null,
                        //     error: this.#getFieldError('dataCollection'),
                        //     onChange: this.#handleChange('dataCollection'),
                        // }}
                        // Form
                        onSubmit={this.#submit}
                        submitDisabled={pb.hasFormErrors(errors)}
                    />
                </div>
            </div>
        );
    }
}

export default function InitSetup() {
    const intl = useIntl();
    const navigate = useNavigate();
    return <View intl={intl} navigate={navigate} />;
}
