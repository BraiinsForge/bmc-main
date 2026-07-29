// Copyright (C) 2025  Braiins Systems s.r.o.
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

// Libs
import { Helmet } from '@dr.pogodin/react-helmet';
import { debounce } from 'es-toolkit';
import { FormattedMessage, type IntlShape, useIntl } from 'react-intl';
import { useNavigate, type NavigateFunction } from 'react-router';

import { setState } from '@/lib/react';
import { toast } from '@/lib/toast';
import type { FormPropsToValuesRec, iField } from '@/lib/form';

// App
import * as pb from '@/proto';
import { URLS } from '@/constants';
import { getID } from './const';
import AppContext, { type AppContextType } from '@/context';

// Components
import * as Comp from './components';
import { Button, Modal, type FieldValue } from '@/components';
import { Add as IconAdd } from '@carbon/react/icons';

// CSS
import css from './Accounts.scss';

interface Props {
    intl: IntlShape;
    navigate: NavigateFunction;
}

type Dialog = null | { mode: 'create' } | { mode: 'edit'; id: string; typeId: string };
// Error shape mirroring the request payload's field paths,
// so `parseFormErrors` routes each violation back to its control:
// `name`, `typeId`, and the per-field `fieldValues["<key>"]`.
type AccountErrors = {
    name: string;
    typeId: string;
    fieldValues: Record<string, string>;
};
type FormState = {
    values: FormPropsToValuesRec<Pick<Comp.AccountFormProps, 'type' | 'name'>>;
    fieldValues: Record<string, FieldValue>;
    errors: null | pb.FormErrors<AccountErrors>;
};

interface State {
    accounts: pb.Account[];
    credentialTypes: pb.CredentialTypeLookup;
    isLoading: boolean;
    isSaving: boolean;
    dialog: Dialog;
    form: FormState;
}

const emptyForm = (): FormState => ({ values: { type: '', name: '' }, errors: null, fieldValues: {} });
const getInitialState = (): State => ({
    accounts: [],
    credentialTypes: new Map(),
    isLoading: false,
    isSaving: false,
    dialog: null,
    form: emptyForm(),
});

const $ = getID('list').get;
class View extends Component<Props, State> {
    static contextType = AppContext;
    declare context: AppContextType;

    readonly state = getInitialState();
    componentDidMount = () => this.#mount();
    componentWillUnmount = () => pb.abort.all(this);
    #mount = debounce(() => this.#fetch(), 150);

    get #txt() {
        const { formatMessage } = this.props.intl;
        return {
            title: formatMessage({ defaultMessage: 'Connected Accounts' }),
            subtitle: formatMessage({ defaultMessage: 'Manage your API credentials and service accounts.' }),
            add: formatMessage({ defaultMessage: 'Add New Account' }),
            edit: formatMessage({ defaultMessage: 'Edit Account' }),
            cancel: formatMessage({ defaultMessage: 'Cancel' }),
            save: formatMessage({ defaultMessage: 'Save' }),
            empty: formatMessage({ defaultMessage: 'No Connected Accounts Yet.' }),
        };
    }

    private fetchAbort = pb.abort.get();
    #fetch = async (): Promise<void> => {
        const { formatMessage } = this.props.intl;
        await setState(this, { isLoading: true });

        try {
            const opts = this.fetchAbort.replace();
            const [accountsResponse, typesResponse] = await Promise.all([
                pb.rpc.accounts.getAllAccounts({}, opts),
                pb.rpc.credentials.getCredentialTypes({}, opts),
            ]);
            this.setState({
                accounts: accountsResponse.accounts,
                credentialTypes: new Map(typesResponse.credentialTypes.map(t => [t.id, t])),
            });
        } catch ($) {
            if (pb.abort.is($)) return;

            let msg = pb.collectAllErrorsAsFormattedList($);
            msg ||= formatMessage({ defaultMessage: 'Failed to load accounts!' });
            toast.error(msg);
        } finally {
            this.setState({ isLoading: false });
        }
    };

    //
    // Dialog
    //

    #openCreate = (): void => {
        const first = this.state.credentialTypes.values().next().value;
        this.setState({
            dialog: { mode: 'create' },
            form: { ...emptyForm(), values: { type: first?.id ?? '', name: '' } },
        });
    };
    #openEdit = (acc: pb.Account): void => {
        this.setState({
            dialog: { mode: 'edit', id: acc.id, typeId: acc.typeId },
            form: { ...emptyForm(), values: { type: acc.typeId, name: acc.name } },
        });
    };
    #close = (): void => this.setState({ dialog: null });

    #onType = (type: string): void =>
        this.setState(s => ({
            form: { ...s.form, errors: null, values: { ...s.form.values, type }, fieldValues: {} },
        }));
    #onName = (name: string): void =>
        this.setState(s => ({ form: { ...s.form, errors: null, values: { ...s.form.values, name } } }));
    #onField = (key: string, value: FieldValue): void =>
        this.setState(s => ({
            form: { ...s.form, errors: null, fieldValues: { ...s.form.fieldValues, [key]: value } },
        }));

    private submitAbort = pb.abort.get();
    #submit = async (): Promise<void> => {
        const { formatMessage } = this.props.intl;
        const { dialog, form } = this.state;
        if (!dialog) return;

        await setState(this, { isSaving: true });

        try {
            const opts = this.submitAbort.replace();

            // A blank field is omitted; on edit an empty map means "keep the stored secrets".
            const fieldValues: Record<string, string> = {};
            for (const [key, value] of Object.entries(form.fieldValues)) {
                const text = typeof value === 'string' ? value : value == null ? '' : String(value);
                if (text.length > 0) fieldValues[key] = text;
            }

            const request = pb.create(pb.UpsertAccountRequestSchema, {
                id: dialog.mode === 'edit' ? dialog.id : '',
                typeId: dialog.mode === 'create' ? (form.values.type ?? '') : '',
                name: form.values.name ?? '',
                fieldValues,
            });
            await pb.rpc.accounts.upsertAccount(request, opts);

            this.#close();
            toast.success(
                formatMessage({ defaultMessage: 'Account "{name}" saved' }, { name: form.values.name ?? '' }),
            );
        } catch ($) {
            if (pb.abort.is($)) return;

            const errors = pb.parseFormErrors<AccountErrors>($, ['name', 'typeId', 'fieldValues']);
            this.setState(s => ({ form: { ...s.form, errors } }));
        } finally {
            this.setState({ isSaving: false }, this.#fetch);
        }
    };

    #delete = async (acc: pb.Account): Promise<void> => {
        const {
            intl: { formatMessage },
            navigate,
        } = this.props;
        const { confirm } = this.context;

        const title = formatMessage({ defaultMessage: 'Delete Account' });
        const cancel = formatMessage({ defaultMessage: 'Cancel' });
        const connected = acc.connectedWidgets.length;

        // A bound account can't be deleted until its widgets are freed.
        if (connected > 0) {
            const answer = await confirm({
                size: 'sm',
                danger: false,
                title,
                cancelLabel: cancel,
                confirmLabel: formatMessage({ defaultMessage: 'Go to display widgets' }),
                message: (
                    <FormattedMessage
                        defaultMessage="This account is linked to <b>{count, plural, one {1 widget} other {# widgets}}</b>. To delete it, please remove or edit {count, plural, one {that widget} other {those widgets}}."
                        values={{ b: ch => <strong children={ch} />, count: connected }}
                    />
                ),
            });
            if (answer) navigate(URLS.pages.display.list);
            return;
        }

        const confirmed = await confirm({
            size: 'sm',
            danger: true,
            title,
            cancelLabel: cancel,
            confirmLabel: title,
            message: (
                <FormattedMessage
                    defaultMessage="This account isn’t used in any display widgets. You can safely delete {name} now."
                    values={{ name: <strong children={acc.name} /> }}
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
            this.#fetch();
        }
    };

    //
    // Render
    //

    #renderAddButton = (): ReactElement => (
        <Button
            id={$('connect-acc')}
            key="connect-acc"
            kind="primary"
            onClick={this.#openCreate}
            icon={IconAdd}
            children={this.#txt.add}
        />
    );

    #renderModal = (): ReactNode => {
        const { dialog, form, credentialTypes, isSaving } = this.state;
        if (!dialog) return null;

        const txt = this.#txt;
        const isEdit = dialog.mode === 'edit';

        const errors = form.errors;
        const type: iField<string> = {
            value: form.values.type ?? '',
            onChange: this.#onType,
            error: pb.renderFieldErrorsAsList(errors?.fields?.typeId),
        };
        const name: iField<string> = {
            value: form.values.name ?? '',
            onChange: this.#onName,
            error: pb.renderFieldErrorsAsList(errors?.fields?.name),
        };

        return (
            <Modal
                id={$('account-modal')}
                open
                size="sm"
                modalHeading={isEdit ? txt.edit : txt.add}
                selectorPrimaryFocus="input"
                onRequestSubmit={this.#submit}
                primaryButtonText={isEdit ? txt.save : txt.add}
                primaryButtonDisabled={isSaving}
                onSecondarySubmit={this.#close}
                onRequestClose={this.#close}
                secondaryButtonText={txt.cancel}
            >
                <Comp.AccountForm
                    mode={dialog.mode}
                    credentialTypes={credentialTypes}
                    type={type}
                    name={name}
                    fieldValues={form.fieldValues}
                    fieldErrors={errors?.fields?.fieldValues}
                    onFieldChange={this.#onField}
                    error={pb.renderFieldErrorsAsList(errors?.global)}
                />
            </Modal>
        );
    };

    render() {
        const { accounts, credentialTypes } = this.state;
        const txt = this.#txt;

        let content: ReactNode;
        if (accounts.length > 0) {
            content = (
                <Comp.ConnectedAccountsTable
                    accounts={accounts}
                    credentialTypes={credentialTypes}
                    onEdit={this.#openEdit}
                    onDelete={this.#delete}
                />
            );
        } else {
            content = (
                <div className={css.emptyViewWrapper}>
                    <Comp.Placeholder rowsCount={3} className={css.placeholderTable} />
                    <h2 className={css.heading} children={txt.empty} />
                    {this.#renderAddButton()}
                </div>
            );
        }

        return (
            <div className={css.root}>
                <Helmet title={txt.title} />
                <header className={css.header}>
                    <div className={css.headerLeft}>
                        <h1 className={css.title} children={txt.title} />
                        <div className={css.subtitle} children={txt.subtitle} />
                    </div>
                    <div className={css.headerControls} children={this.#renderAddButton()} />
                </header>

                <main className={css.main} children={content} />

                {this.#renderModal()}
            </div>
        );
    }
}

export default function ApiPage() {
    const intl = useIntl();
    const navigate = useNavigate();

    return <View intl={intl} navigate={navigate} />;
}
