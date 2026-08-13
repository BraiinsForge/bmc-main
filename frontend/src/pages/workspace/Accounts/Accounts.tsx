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

import { Component, Fragment } from 'react';

// Libs
import { Helmet } from '@dr.pogodin/react-helmet';
import { debounce } from 'es-toolkit';
import { FormattedMessage, type IntlShape, useIntl } from 'react-intl';

import { setState } from '@/lib/react';
import { toast } from '@/lib/toast';
import type { FormPropsToValuesRec, iField } from '@/lib/form';

// App
import * as pb from '@/proto';
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
}

type Dialog = null | { mode: 'create' } | { mode: 'edit'; id: string; typeId: string };
// Error shape mirroring the request payload's field paths,
// so `parseFormErrors` routes each violation back to its control:
// `name`, `typeId`, and the per-field `fieldValues["<key>"]`.
type AccountErrors = {
    name: string;
    typeId: string;
    fieldValues: Record<string, string>;
    allowHosts: string;
};
type FormState = {
    values: FormPropsToValuesRec<Pick<Comp.AccountFormProps, 'type' | 'name' | 'allowHosts'>>;
    fieldValues: Record<string, FieldValue>;
    errors: null | pb.FormErrors<AccountErrors>;
};

/// One destination per line, blank lines dropped — the server rejects an
/// empty entry, and a stray newline is not something to fail a save over.
/// The submit writes the normalized text back into the textarea before
/// sending, so the server's per-line errors number the lines on screen.
const splitAllowHosts = (text: string): string[] =>
    text
        .split('\n')
        .map(line => line.trim())
        .filter(Boolean);

interface State {
    accounts: pb.Account[];
    credentialTypes: pb.CredentialTypeLookup;
    isLoading: boolean;
    isSaving: boolean;
    dialog: Dialog;
    form: FormState;
}

const emptyForm = (): FormState => ({
    values: { type: '', name: '', allowHosts: '' },
    errors: null,
    fieldValues: {},
});
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
            form: { ...emptyForm(), values: { type: first?.id ?? '', name: '', allowHosts: '' } },
        });
    };
    #openEdit = (acc: pb.Account): void => {
        this.setState({
            dialog: { mode: 'edit', id: acc.id, typeId: acc.typeId },
            form: {
                ...emptyForm(),
                // Unlike the secrets, the stored list comes back on read,
                // so the textarea opens on what is actually in force.
                values: { type: acc.typeId, name: acc.name, allowHosts: acc.allowHosts.join('\n') },
            },
        });
    };
    #close = (): void => this.setState({ dialog: null });

    // Destinations reset with the fields: a pinned type renders no control to clear them.
    #onType = (type: string): void =>
        this.setState(s => ({
            form: {
                ...s.form,
                errors: null,
                values: { ...s.form.values, type, allowHosts: '' },
                fieldValues: {},
            },
        }));
    #onName = (name: string): void =>
        this.setState(s => ({ form: { ...s.form, errors: null, values: { ...s.form.values, name } } }));
    #onAllowHosts = (allowHosts: string): void =>
        this.setState(s => ({ form: { ...s.form, errors: null, values: { ...s.form.values, allowHosts } } }));
    #onField = (key: string, value: FieldValue): void =>
        this.setState(s => ({
            form: { ...s.form, errors: null, fieldValues: { ...s.form.fieldValues, [key]: value } },
        }));

    private submitAbort = pb.abort.get();
    #submit = async (): Promise<void> => {
        const { formatMessage } = this.props.intl;
        const { dialog, form } = this.state;
        if (!dialog) return;

        // Covers a hand-edited store, whose list never passed through the form.
        const pinned = !!this.state.credentialTypes.get(form.values.type ?? '')?.egress?.allowHosts.length;
        const hosts = pinned ? [] : splitAllowHosts(form.values.allowHosts ?? '');

        await setState(this, {
            isSaving: true,
            form: pinned ? form : { ...form, values: { ...form.values, allowHosts: hosts.join('\n') } },
        });

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
                allowHosts: hosts,
            });
            await pb.rpc.accounts.upsertAccount(request, opts);

            this.#close();
            toast.success(
                formatMessage({ defaultMessage: 'Account "{name}" saved' }, { name: form.values.name ?? '' }),
            );
        } catch ($) {
            if (pb.abort.is($)) return;

            const errors = pb.parseFormErrors<AccountErrors>($, ['name', 'typeId', 'fieldValues', 'allowHosts']);
            this.setState(s => ({ form: { ...s.form, errors } }));
        } finally {
            this.setState({ isSaving: false }, this.#fetch);
        }
    };

    // Names for the widgets an account is bound to,
    // resolved from the scenes and the widget catalog.
    //
    // Best effort: a failed lookup costs the list, not the delete.
    // An abort rethrows, since the caller must not then open a dialog.
    private boundWidgetsAbort = pb.abort.get();
    #boundWidgetNames = async (ids: string[]): Promise<string[]> => {
        const bound = new Set<string>(ids);
        const found: string[] = [];

        try {
            const opts = this.boundWidgetsAbort.replace();
            const [{ scenes }, { widgets }] = await Promise.all([
                pb.rpc.scenes.getScenes({}, opts),
                pb.rpc.scenes.getAvailableWidgets({}, opts),
            ]);
            const names = new Map(widgets.map(w => [w.uid, w.name]));

            for (const { kind } of scenes) {
                const placed =
                    kind.case === 'combined'
                        ? kind.value.widgets
                        : kind.case === 'fullscreen' && kind.value.widget
                          ? [kind.value.widget]
                          : [];

                for (const widget of placed) {
                    if (!bound.has(widget.id)) continue;
                    const name = widget.config && names.get(widget.config.widgetUid);
                    // A widget whose type is no longer installed resolves to no name;
                    // dropping it would leave this list shorter than the count beside it.
                    found.push(
                        name ||
                            this.props.intl.formatMessage(
                                { defaultMessage: 'Unknown widget ({id})' },
                                { id: widget.config?.widgetUid || widget.id },
                            ),
                    );
                }
            }
        } catch ($) {
            if (pb.abort.is($)) throw $;

            // The count still stands, so the delete is still offered.
            // Say why the list is missing, or its absence reads as "nothing affected".
            let msg = pb.collectAllErrorsAsFormattedList($);
            msg ||= this.props.intl.formatMessage({
                defaultMessage: 'Could not list the widgets using this account.',
            });
            toast.error(msg);
        }

        return found;
    };

    #delete = async (acc: pb.Account): Promise<void> => {
        const {
            intl: { formatMessage },
        } = this.props;
        const { confirm } = this.context;

        const title = formatMessage({ defaultMessage: 'Delete Account' });
        const cancel = formatMessage({ defaultMessage: 'Cancel' });
        const connected = acc.connectedWidgets.length;
        // Deleting unbinds everywhere in one server-side step, so the dialog's job
        // is to say what that will affect rather than to send the operator away.
        let names: string[] = [];
        if (connected > 0) {
            try {
                names = await this.#boundWidgetNames(acc.connectedWidgets);
            } catch ($) {
                if (pb.abort.is($)) return;
            }
        }

        const confirmed = await confirm({
            size: 'sm',
            danger: true,
            title,
            cancelLabel: cancel,
            confirmLabel: title,
            message:
                connected > 0 ? (
                    <Fragment>
                        <FormattedMessage
                            defaultMessage="Deleting {name} will unbind it from <b>{count, plural, one {1 widget} other {# widgets}}</b>, which will stop using it."
                            values={{
                                b: ch => <strong children={ch} />,
                                name: <strong children={acc.name} />,
                                count: connected,
                            }}
                        />
                        {names.length > 0 ? (
                            <ul
                                className={css.affectedWidgets}
                                // Indexed: two widgets of one type share a name.
                                children={names.map((n, i) => <li key={`${n}-${i}`} children={n} />)}
                            />
                        ) : null}
                    </Fragment>
                ) : (
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
        const allowHosts: iField<string> = {
            value: form.values.allowHosts ?? '',
            onChange: this.#onAllowHosts,
            // Frozen while a save is in flight, so a server "Line N" error
            // cannot arrive numbering lines the operator has since moved.
            disabled: this.state.isSaving,
            error: pb.renderFieldErrorsAsList(errors?.fields?.allowHosts),
        };

        return (
            <Modal
                id={$('account-modal')}
                open
                size="sm"
                modalHeading={isEdit ? txt.edit : txt.add}
                selectorPrimaryFocus="input[type='radio'],input[type='text']"
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
                    allowHosts={allowHosts}
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

    return <View intl={intl} />;
}
