import { Component, Fragment } from 'react';
import { type iField, getID } from '@/lib/form';
import { useIntl, type IntlShape } from 'react-intl';

import * as pb from '@/proto';

import { FormPasswordChange } from '../FormPasswordChange';
import { Modal, Button, ButtonGroup, InlineNotificationsGroup, Field, FieldSet } from '@/components';
import { Toggle } from '@carbon/react';

// Styles
import css from './SectionSecurity.scss';

export interface SectionSecurityProps {
    hasPassword: null | boolean;

    actions: null | {
        onPasswordChange(d: pb.ChangePasswordRequest): Promise<void>;
        onPasswordRemove(d: pb.RemovePasswordRequest): Promise<void>;
        onPasswordCreate(d: pb.CreatePasswordRequest): Promise<void>;
    };

    dataCollection: iField<boolean>;
}
interface Props extends SectionSecurityProps {
    intl: IntlShape;
}

enum PasswordDialogKind {
    change = 'change',
    remove = 'remove',
    create = 'create',
}

interface State {
    hasPassword: null | boolean;
    openDialog: null | PasswordDialogKind;

    passRemove: pb.FormState<pb.RemovePasswordRequest>;
    passCreate: pb.FormState<pb.CreatePasswordRequest, { newPasswordConfirm: string }>;
    passChange: pb.FormState<pb.ChangePasswordRequest, { newPasswordConfirm: string }>;
}
const getInitialState = (): State => ({
    hasPassword: null,
    openDialog: null,

    passRemove: {
        values: { password: '' },
        errors: null,
    },
    passCreate: {
        values: { password: '', newPasswordConfirm: '' },
        errors: null,
    },
    passChange: {
        values: { currentPassword: '', newPassword: '', newPasswordConfirm: '' },
        errors: null,
    },
});

const $id = getID('settings', 'security');

class View extends Component<Props, State> {
    readonly state = getInitialState();

    get #txt() {
        const { formatMessage } = this.props.intl;
        return {
            cancel: formatMessage({ defaultMessage: 'Cancel' }),
            requiredField: formatMessage({ defaultMessage: 'Required field!' }),
            passwordsMustMatch: formatMessage({ defaultMessage: 'Passwords must match!' }),
        };
    }

    #passChangeToggle = (): void => {
        const openDialog: State['openDialog'] = this.state.openDialog == null ? PasswordDialogKind.change : null;
        this.setState({ openDialog });
    };
    #passChangeUpdate = <Key extends keyof State['passChange']['values']>(key: Key) => {
        return (value: string): void => {
            this.setState(s => ({
                passChange: {
                    errors: null,
                    values: {
                        ...s.passChange.values,
                        [key]: value,
                    },
                },
            }));
        };
    };
    #passChangeSubmit = async (): Promise<void> => {
        const { actions } = this.props;
        const d = this.state.passChange.values;
        const txt = this.#txt;
        const errors: Required<State['passChange']['errors']> = {
            global: [],
            fields: {},
        };

        if (!d.currentPassword) errors.fields.currentPassword = [txt.requiredField];
        if (!d.newPassword) errors.fields.newPassword = [txt.requiredField];
        if (!d.newPasswordConfirm) errors.fields.newPasswordConfirm = [txt.requiredField];
        if (d.newPassword !== d.newPasswordConfirm) errors.fields.newPasswordConfirm = [txt.passwordsMustMatch];

        // Abort if we have errors
        if (pb.hasFormErrors(errors)) {
            return this.setState(s => ({
                passChange: {
                    ...s.passChange,
                    errors,
                },
            }));
        }

        try {
            await actions?.onPasswordChange(pb.create(pb.ChangePasswordRequestSchema, d));
            this.#passDialogCancel();
        } catch ($) {
            if (pb.abort.is($)) return;
            const errors = pb.parseFormErrors<pb.ChangePasswordRequest>($, ['currentPassword', 'newPassword']);
            this.setState(s => ({ passChange: { ...s.passChange, errors } }));
        }
    };

    #passRemoveToggle = (): void => {
        const openDialog: State['openDialog'] = this.state.openDialog == null ? PasswordDialogKind.remove : null;
        this.setState({ openDialog });
    };
    #passRemoveUpdate = <Key extends pb.MessageFields<pb.RemovePasswordRequest>>(key: Key) => {
        return (value: string): void => {
            this.setState(s => ({
                passRemove: {
                    errors: null,
                    values: {
                        ...s.passRemove.values,
                        [key]: value,
                    },
                },
            }));
        };
    };
    #passRemoveSubmit = async (): Promise<void> => {
        const { actions } = this.props;
        const d = this.state.passRemove.values;
        const txt = this.#txt;
        const errors: Required<State['passRemove']['errors']> = {
            global: [],
            fields: {},
        };

        if (!d.password) errors.fields.password = [txt.requiredField];

        // Abort if we have errors
        if (pb.hasFormErrors(errors)) {
            return this.setState(s => ({ passRemove: { ...s.passRemove, errors } }));
        }

        try {
            await actions?.onPasswordRemove(pb.create(pb.RemovePasswordRequestSchema, d));
            this.#passDialogCancel();
        } catch ($) {
            if (pb.abort.is($)) return;
            const errors = pb.parseFormErrors<pb.RemovePasswordRequest>($, ['password']);
            this.setState(s => ({ passRemove: { ...s.passRemove, errors } }));
        }
    };

    #passCreateToggle = (): void => {
        const openDialog: State['openDialog'] = this.state.openDialog == null ? PasswordDialogKind.create : null;
        this.setState({ openDialog });
    };
    #passCreateUpdate = <Key extends keyof State['passCreate']['values']>(key: Key) => {
        return (value: string): void => {
            this.setState(s => ({
                passCreate: {
                    errors: null,
                    values: {
                        ...s.passCreate.values,
                        [key]: value,
                    },
                },
            }));
        };
    };
    #passCreateSubmit = async (): Promise<void> => {
        const { actions } = this.props;
        const d = this.state.passCreate.values;
        const txt = this.#txt;
        const errors: Required<State['passCreate']['errors']> = {
            global: [],
            fields: {},
        };

        if (!d.password) errors.fields.password = [txt.requiredField];
        if (!d.newPasswordConfirm) errors.fields.newPasswordConfirm = [txt.requiredField];
        if (d.password !== d.newPasswordConfirm) errors.fields.newPasswordConfirm = [txt.passwordsMustMatch];

        // Abort if we have errors
        if (pb.hasFormErrors(errors)) {
            return this.setState(s => ({ passCreate: { ...s.passCreate, errors } }));
        }

        try {
            await actions?.onPasswordCreate(pb.create(pb.CreatePasswordRequestSchema, d));
            this.#passDialogCancel();
        } catch ($) {
            if (pb.abort.is($)) return;
            const errors = pb.parseFormErrors<pb.CreatePasswordRequest>($, ['password']);
            this.setState(s => ({ passCreate: { ...s.passCreate, errors } }));
        }
    };

    #passDialogCancel = () => {
        const { passChange, passCreate, passRemove } = getInitialState();
        this.setState({
            openDialog: null,
            passChange,
            passCreate,
            passRemove,
        });
    };
    #passDialogRender = (): ReactNode => {
        const { formatMessage } = this.props.intl;
        const { openDialog, passCreate, passRemove, passChange } = this.state;

        const txt = this.#txt;
        let isDanger: boolean = false;
        let submitFn: undefined | Fn;
        let submitText: string = '';
        let globalErrors: Maybe<string[]>;
        let hasErrors: boolean = false;
        let content: ReactNode;

        switch (openDialog) {
            case PasswordDialogKind.change:
                submitFn = this.#passChangeSubmit;
                submitText = formatMessage({ defaultMessage: 'Change Password' });
                globalErrors = passChange.errors?.global;
                hasErrors = pb.hasFormErrors(passChange.errors);
                content = (
                    <Fragment>
                        <p
                            className={css.passDialogIntro}
                            children={formatMessage({
                                defaultMessage:
                                    'Enter your current password and a new password to update your credentials.',
                            })}
                        />

                        <FormPasswordChange
                            passCurrent={{
                                value: passChange.values.currentPassword,
                                error: pb.renderFieldErrorsAsList(passChange.errors?.fields?.currentPassword),
                                onChange: this.#passChangeUpdate('currentPassword'),
                            }}
                            passNew={{
                                value: passChange.values.newPassword,
                                error: pb.renderFieldErrorsAsList(passChange.errors?.fields?.newPassword),
                                onChange: this.#passChangeUpdate('newPassword'),
                            }}
                            passConfirm={{
                                value: passChange.values.newPasswordConfirm,
                                error: pb.renderFieldErrorsAsList(passChange.errors?.fields?.newPasswordConfirm),
                                onChange: this.#passChangeUpdate('newPasswordConfirm'),
                            }}
                        />
                    </Fragment>
                );
                break;

            case PasswordDialogKind.create:
                submitFn = this.#passCreateSubmit;
                submitText = formatMessage({ defaultMessage: 'Create Password' });
                globalErrors = passCreate.errors?.global;
                hasErrors = pb.hasFormErrors(passCreate.errors);
                content = (
                    <FormPasswordChange
                        passCurrent={null}
                        passNew={{
                            value: passCreate.values.password,
                            error: pb.renderFieldErrorsAsList(passCreate.errors?.fields?.password),
                            onChange: this.#passCreateUpdate('password'),
                        }}
                        passConfirm={{
                            value: passCreate.values.newPasswordConfirm,
                            error: pb.renderFieldErrorsAsList(passCreate.errors?.fields?.newPasswordConfirm),
                            onChange: this.#passCreateUpdate('newPasswordConfirm'),
                        }}
                    />
                );

                break;

            case PasswordDialogKind.remove:
                isDanger = true;
                submitFn = this.#passRemoveSubmit;
                submitText = formatMessage({ defaultMessage: 'Remove Password' });
                globalErrors = passRemove.errors?.global;
                hasErrors = pb.hasFormErrors(passRemove.errors);
                content = (
                    <Fragment>
                        <p
                            className={css.passDialogIntro}
                            children={formatMessage({
                                defaultMessage: 'To remove your password, please enter your current password.',
                            })}
                        />

                        <FormPasswordChange
                            passCurrent={{
                                value: passRemove.values.password,
                                error: pb.renderFieldErrorsAsList(passRemove.errors?.fields?.password),
                                onChange: this.#passRemoveUpdate('password'),
                            }}
                            passNew={null}
                            passConfirm={null}
                        />
                    </Fragment>
                );
                break;
        }

        return (
            <Modal
                id={$id.get('password-change-dialog')}
                size="md"
                open={openDialog != null}
                danger={isDanger}
                modalHeading={submitText}
                // Submit
                onRequestSubmit={submitFn}
                primaryButtonText={submitText}
                primaryButtonDisabled={hasErrors}
                // Cancel
                onRequestClose={this.#passDialogCancel}
                onSecondarySubmit={this.#passDialogCancel}
                secondaryButtonText={txt.cancel}
            >
                <InlineNotificationsGroup
                    items={globalErrors}
                    kind="error"
                    theme="inverse"
                    stretch
                    style={{ marginBottom: 16 }}
                />
                {content}
            </Modal>
        );
    };

    render() {
        const {
            intl: { formatMessage },
            hasPassword,
            actions,

            // Privacy
            dataCollection,
        } = this.props;

        return (
            <section className={css.root}>
                <FieldSet title={formatMessage({ defaultMessage: 'Password' })}>
                    <Field
                        title={formatMessage({ defaultMessage: 'Device Password' })}
                        description={formatMessage({
                            defaultMessage: 'Change the password used to access device settings.',
                        })}
                        disabled={!actions}
                    >
                        {hasPassword ? (
                            <ButtonGroup spaced>
                                <Button
                                    kind="secondary"
                                    children={formatMessage({ defaultMessage: 'Change Password' })}
                                    onClick={this.#passChangeToggle}
                                />
                                <Button
                                    kind="tertiary"
                                    children={formatMessage({ defaultMessage: 'Remove Password' })}
                                    onClick={this.#passRemoveToggle}
                                />
                            </ButtonGroup>
                        ) : (
                            <Button
                                kind="secondary"
                                children={formatMessage({ defaultMessage: 'Create Password' })}
                                onClick={this.#passCreateToggle}
                            />
                        )}
                    </Field>
                </FieldSet>
                {this.#passDialogRender()}

                <FieldSet title={formatMessage({ defaultMessage: 'Privacy' })}>
                    <Field
                        title={formatMessage({ defaultMessage: 'Data Collection' })}
                        description={formatMessage({
                            defaultMessage: 'Allow anonymous usage data collection to improve the product.',
                        })}
                        disabled={dataCollection.disabled}
                    >
                        <Toggle
                            id={$id.get('data-collection')}
                            size="md"
                            toggled={!!dataCollection.value}
                            onToggle={dataCollection.onChange}
                            disabled={dataCollection.disabled}
                            labelA={formatMessage({ defaultMessage: 'Off' })}
                            labelB={formatMessage({ defaultMessage: 'On' })}
                        />
                    </Field>
                </FieldSet>
            </section>
        );
    }
}

export function SectionSecurity(props: SectionSecurityProps) {
    const intl = useIntl();
    return <View {...props} intl={intl} />;
}
