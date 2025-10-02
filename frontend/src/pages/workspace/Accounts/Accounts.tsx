import { Component, Fragment } from 'react';

// Libs
import { Helmet } from '@dr.pogodin/react-helmet';
import { cloneDeep, debounce } from 'es-toolkit';
import { FormattedMessage, type IntlShape, useIntl } from 'react-intl';
import { useNavigate, type NavigateFunction } from 'react-router';

import { setState } from '@/lib/react';
import { assertUnreachable } from '@/lib/ts';
import type { FormPropsToLocalState } from '@/lib/form';
import { toast } from '@/lib/toast';

// App
import * as pb from '@/proto';
import { URLS } from '@/constants';
import { getID } from './const';
import AppContext, { type AppContextType } from '@/context';

// Components
import * as Comp from './components';
import { Button, Modal } from '@/components';
import { Add as IconAdd } from '@carbon/react/icons';

// CSS
import css from './Accounts.scss';

interface Props {
    intl: IntlShape;
    navigate: NavigateFunction;
}

type FormStateCombined = FormPropsToLocalState<Pick<Comp.FormCombinedProps, 'type'>> & {
    connectedWidgetsCount: null | number;
};
type FormStateBraiinsPool = FormPropsToLocalState<Comp.FormBraiinsPoolProps>;

type DialogStates = {
    combined: FormStateCombined;
    braiinsPool: FormStateBraiinsPool;
};
function getInitialDialogStates(): DialogStates {
    return {
        combined: {
            errors: null,
            values: { type: pb.AccountType.BRAIINSPOOL },
            connectedWidgetsCount: null,
        },
        braiinsPool: {
            errors: null,
            values: {
                name: '',
                apiKey: '',
            },
        },
    };
}

interface State {
    accounts: pb.Account[];
    isLoading: boolean;

    isSaving: boolean;
    openDialog: null | { kind: 'create' } | { kind: 'edit'; id: string };
    dialogStates: DialogStates;
}
const getInitialState = (): State => ({
    accounts: [],
    isLoading: false,

    isSaving: false,
    openDialog: null,
    dialogStates: getInitialDialogStates(),
});

const $ = getID('list').get;
class View extends Component<Props, State> {
    static contextType = AppContext;
    declare context: AppContextType;

    readonly state = getInitialState();
    componentDidMount = () => this.#mount();
    componentWillUnmount = () => pb.abort.all(this);

    #mount = debounce(() => this.#fetchAccounts(), 150);
    get #txt() {
        const { formatMessage } = this.props.intl;
        return {
            title: formatMessage({ defaultMessage: 'Connected Accounts' }),
            cancel: formatMessage({ defaultMessage: 'Cancel' }),
            addNewAccount: formatMessage({ defaultMessage: 'Add New Account' }),
            editAccount: formatMessage({ defaultMessage: 'Edit Account' }),
        };
    }

    private fetchAccountsAbort = pb.abort.get();
    #fetchAccounts = async (): Promise<void> => {
        const { formatMessage } = this.props.intl;
        await setState(this, { isLoading: true });

        try {
            const { accounts } = await pb.rpc.accounts.getAllAccounts({}, this.fetchAccountsAbort.replace());
            this.setState({ accounts, isLoading: false });
        } catch ($) {
            if (pb.abort.is($)) return;

            let msg = pb.collectAllErrorsAsFormattedList($);
            msg ||= formatMessage({ defaultMessage: 'Failed to fetch accounts!' });
            toast.error(msg);
        } finally {
            this.setState({ isLoading: false });
        }
    };

    #getFieldChangeHandler = <
        const Kind extends keyof DialogStates,
        const FieldKey extends keyof DialogStates[Kind]['values'],
    >(
        widgetKind: Kind,
        fieldKey: FieldKey,
    ) => {
        return (value: DialogStates[Kind]['values'][FieldKey]) => {
            this.setState(s => {
                const form = cloneDeep(s.dialogStates[widgetKind]);
                form.errors = null;
                form.values = {
                    ...form.values,
                    [fieldKey]: value,
                };

                return {
                    dialogStates: {
                        ...s.dialogStates,
                        [widgetKind]: form,
                    },
                };
            });
        };
    };
    #getFieldValue = <const Kind extends keyof DialogStates, const FieldKey extends keyof DialogStates[Kind]['values']>(
        widgetKind: Kind,
        fieldKey: FieldKey,
    ) => {
        const { dialogStates } = this.state;

        const values = dialogStates[widgetKind].values as DialogStates[Kind]['values'];
        return values?.[fieldKey] ?? null;
    };
    #getFieldError = <const Kind extends keyof DialogStates, const FieldKey extends keyof DialogStates[Kind]['values']>(
        widgetKind: Kind,
        fieldKey: FieldKey,
    ): null | string => {
        const { dialogStates } = this.state;

        const errors = dialogStates[widgetKind].errors as null | pb.FormErrors<any>;
        if (!errors) return null;

        const fieldError = errors.fields?.[fieldKey] as null | pb.FieldErrors;
        return pb.renderFieldErrorsAsList(fieldError);
    };
    #getFieldStruct = <
        const Kind extends keyof DialogStates,
        const FieldKey extends keyof DialogStates[Kind]['values'],
    >(
        widgetKind: Kind,
        fieldKey: FieldKey,
    ) => {
        return {
            value: this.#getFieldValue(widgetKind, fieldKey),
            error: this.#getFieldError(widgetKind, fieldKey),
            onChange: this.#getFieldChangeHandler(widgetKind, fieldKey),
            disabled: false,
        };
    };

    //
    // Dialogs
    //

    #dialogClose = (): void => this.setState({ openDialog: null });

    #accountFormOpen = async (): Promise<void> => {
        const { formatMessage } = this.props.intl;

        try {
            const { defaultAccountType } = await pb.rpc.accounts.addAccount({});
            this.setState(s => ({
                openDialog: { kind: 'create' },
                dialogStates: {
                    ...s.dialogStates,
                    combined: {
                        errors: null,
                        connectedWidgetsCount: null,
                        values: { type: defaultAccountType },
                    },
                },
            }));
        } catch ($) {
            let msg = pb.collectAllErrorsAsFormattedList($);
            msg ||= formatMessage({ defaultMessage: 'Failed to load account creation metadata!' });
            toast.error(msg);
        }
    };

    private accountFormSubmitAbort = pb.abort.get();
    #accountFormSubmitNew = async (): Promise<void> => {
        const { formatMessage } = this.props.intl;
        const { openDialog, dialogStates } = this.state;

        if (openDialog?.kind !== 'create') {
            toast.error(formatMessage({ defaultMessage: 'Invalid state, account form is not active!' }));
            return;
        }

        await setState(this, { isSaving: true });

        try {
            const opts = this.accountFormSubmitAbort.replace();

            const accountType = dialogStates.combined.values.type;
            const accountName = dialogStates.braiinsPool.values.name;
            const authentication: pb.Authentication = pb.create(pb.AuthenticationSchema, {
                value: {
                    case: 'apiKey',
                    value: String(dialogStates.braiinsPool.values.apiKey),
                },
            });

            const payload = pb.create(pb.ConnectAppRequestSchema, { accountType, accountName, authentication });
            await pb.rpc.accounts.connectApp(payload, opts);

            this.#dialogClose();
            toast.success(formatMessage({ defaultMessage: 'Account "{name}" connected' }, { name: accountName }));
        } catch ($) {
            if (pb.abort.is($)) return;

            const { global, fields } = pb.parseFormErrors($, ['accountName', 'authentication']);
            const res = cloneDeep(this.state.dialogStates);
            const type = dialogStates.combined.values.type;

            switch (type) {
                case undefined:
                case pb.AccountType.UNSPECIFIED:
                    res.combined.errors = { global };
                    break;

                case pb.AccountType.BRAIINSPOOL: {
                    res.braiinsPool.errors = {
                        global,
                        fields: {
                            name: fields.accountName,
                            apiKey: fields.authentication,
                        },
                    };
                    break;
                }

                default:
                    assertUnreachable(type, 'Unknown account type');
            }

            this.setState({ dialogStates: res });
        } finally {
            this.setState({ isSaving: false }, this.#fetchAccounts);
        }
    };
    #accountFormSubmitEdit = async (): Promise<void> => {
        const { formatMessage } = this.props.intl;
        const { openDialog, dialogStates } = this.state;

        if (openDialog?.kind !== 'edit') {
            toast.error(formatMessage({ defaultMessage: 'Invalid state, account form is not active!' }));
            return;
        }

        await setState(this, { isSaving: true });

        try {
            const opts = this.accountFormSubmitAbort.replace();

            const accountName = dialogStates.braiinsPool.values.name;
            const authentication: pb.Authentication = pb.create(pb.AuthenticationSchema, {
                value: {
                    case: 'apiKey',
                    value: String(dialogStates.braiinsPool.values.apiKey),
                },
            });

            const payload = pb.create(pb.EditAccountRequestSchema, {
                id: openDialog.id,
                accountName,
                authentication,
            });
            await pb.rpc.accounts.editAccount(payload, opts);

            this.#dialogClose();
            toast.success(formatMessage({ defaultMessage: 'Account "{name}" saved' }, { name: accountName }));
        } catch ($) {
            if (pb.abort.is($)) return;

            const { global, fields } = pb.parseFormErrors($, ['accountName', 'authentication']);
            const res = cloneDeep(this.state.dialogStates);
            const type = dialogStates.combined.values.type;

            switch (type) {
                case undefined:
                case pb.AccountType.UNSPECIFIED:
                    res.combined.errors = { global };
                    break;

                case pb.AccountType.BRAIINSPOOL: {
                    res.braiinsPool.errors = {
                        global,
                        fields: {
                            name: fields.accountName,
                            apiKey: fields.authentication,
                        },
                    };
                    break;
                }

                default:
                    assertUnreachable(type, 'Unknown account type');
            }

            this.setState({ dialogStates: res });
        } finally {
            this.setState({ isSaving: false }, this.#fetchAccounts);
        }
    };

    #accountFormRender = (): ReactNode => {
        const { openDialog, dialogStates } = this.state;

        const txt = this.#txt;
        const actionLabel: string = openDialog?.kind === 'edit' ? txt.editAccount : txt.addNewAccount;

        let isOpen: boolean = false;
        let submitMethod: undefined | AnyFunction;
        if (openDialog?.kind === 'create') {
            isOpen = true;
            submitMethod = this.#accountFormSubmitNew;
        } else if (openDialog?.kind === 'edit') {
            isOpen = true;
            submitMethod = this.#accountFormSubmitEdit;
        }

        return (
            <Fragment>
                <Modal
                    id={$('account-modal')}
                    open={isOpen}
                    size="sm"
                    modalHeading={actionLabel}
                    selectorPrimaryFocus="input"
                    // Submit
                    onRequestSubmit={submitMethod}
                    primaryButtonText={actionLabel}
                    primaryButtonDisabled={false} // FIXME: Likely disabled when required fields are empty
                    // Cancel
                    onSecondarySubmit={this.#dialogClose}
                    onRequestClose={this.#dialogClose}
                    secondaryButtonText={txt.cancel}
                >
                    <Comp.FormCombined
                        type={this.#getFieldStruct('combined', 'type')}
                        valuesBraiinsPool={{
                            name: this.#getFieldStruct('braiinsPool', 'name'),
                            apiKey: this.#getFieldStruct('braiinsPool', 'apiKey'),
                        }}
                        connectedWidgetsCount={dialogStates.combined.connectedWidgetsCount}
                    />
                </Modal>
            </Fragment>
        );
    };

    //
    // Header
    //

    #renderAddNewButton = () => {
        const { addNewAccount } = this.#txt;
        return (
            <Button
                id={$('connect-acc')}
                key="connect-acc"
                kind="primary"
                onClick={this.#accountFormOpen}
                icon={IconAdd}
                children={addNewAccount}
            />
        );
    };
    #headerRender = (): ReactElement => {
        return <div className={css.headerControls} children={this.#renderAddNewButton()} />;
    };

    //
    // Table methods
    //

    #edit = async (acc: pb.Account): Promise<void> => {
        this.setState(s => ({
            openDialog: { kind: 'edit', id: acc.id },
            dialogStates: {
                ...s.dialogStates,
                braiinsPool: {
                    errors: null,
                    values: {
                        name: acc.accountName,
                        apiKey: acc.authentication?.value.value,
                    },
                },
                combined: {
                    errors: null,
                    values: { type: acc.accountType },
                    connectedWidgetsCount: acc.connectedWidgets.length,
                },
            },
        }));
    };
    #delete = async (acc: pb.Account): Promise<void> => {
        const {
            intl: { formatMessage },
            navigate,
        } = this.props;
        const { confirm } = this.context;

        const confirmStringTitle: string = formatMessage({ defaultMessage: 'Delete Account' });
        const confirmStringCancel: string = formatMessage({ defaultMessage: 'Cancel' });
        const connectedAccountsCount: number = acc.connectedWidgets.length;

        // When the account is connected to any widget,
        // user has to abort or go edit those widgets
        if (connectedAccountsCount > 0) {
            const answer = await confirm({
                size: 'sm',
                danger: false,
                title: confirmStringTitle,
                cancelLabel: confirmStringCancel,
                confirmLabel: formatMessage({ defaultMessage: 'Go to display scenes' }),
                message: (
                    <FormattedMessage
                        defaultMessage="This account is linked to <b>{count, plural, one {1 widget} other {# widgets}}</b>. To delete it, please remove or edit {count, plural, one {that widget} other {those widgets}}."
                        values={{ b: ch => <strong children={ch} />, count: connectedAccountsCount }}
                    />
                ),
            });
            if (answer) navigate(URLS.pages.display.list);
            return;
        }

        // Otherwise we'll just ask them to confirm the deletion
        const confirmed: boolean = await confirm({
            size: 'sm',
            danger: true,
            title: confirmStringTitle,
            cancelLabel: confirmStringCancel,
            confirmLabel: confirmStringTitle,
            message: (
                <FormattedMessage
                    defaultMessage="This account isn’t used in any display scenes. You can safely delete {name} now."
                    values={{ name: <strong children={acc.accountName} /> }}
                />
            ),
        });
        if (!confirmed) return;

        try {
            await pb.rpc.accounts.removeAccount({ value: acc.id });
            toast.success(formatMessage({ defaultMessage: 'Account deleted' }));
        } catch ($) {
            let msg = pb.collectAllErrorsAsFormattedList($);
            msg ||= formatMessage({ defaultMessage: 'Account deletion failed!' });
            toast.error(msg);
        } finally {
            this.#fetchAccounts();
        }
    };

    render() {
        const { intl } = this.props;
        const { accounts } = this.state;
        const txt = this.#txt;

        let content: ReactNode;

        if (accounts.length > 0) {
            content = <Comp.ConnectedAccountsTable accounts={accounts} onEdit={this.#edit} onDelete={this.#delete} />;
        } else {
            content = (
                <div className={css.emptyViewWrapper}>
                    <Comp.Placeholder rowsCount={3} className={css.placeholderTable} />
                    <h2
                        className={css.heading}
                        children={intl.formatMessage({ defaultMessage: 'No Connected Accounts Yet.' })}
                    />
                    {this.#renderAddNewButton()}
                </div>
            );
        }

        return (
            <div className={css.root}>
                <Helmet title={txt.title} />
                <header className={css.header}>
                    <div className={css.headerLeft}>
                        <h1 className={css.title} children={this.#txt.title} />
                        <div
                            className={css.subtitle}
                            children={intl.formatMessage({
                                defaultMessage: 'Manage your API credentials and service accounts.',
                            })}
                        />
                    </div>

                    {this.#headerRender()}
                </header>

                <main className={css.main} children={content} />

                {this.#accountFormRender()}
            </div>
        );
    }
}

export default function ApiPage() {
    const intl = useIntl();
    const navigate = useNavigate();

    return <View intl={intl} navigate={navigate} />;
}
