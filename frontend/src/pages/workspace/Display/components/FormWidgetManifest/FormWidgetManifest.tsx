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

import { Fragment } from 'react';
import { FormattedMessage, useIntl } from 'react-intl';
import * as pb from '@/proto';
import { URLS } from '@/constants';
import { Form, hasFormErrors } from '@/lib/form';
import { getID } from '../const';
import type { FormifiedParams, FormifiedValue, ParamsFormErrors } from '../../fn';

import { ParamField } from '@/components/ParamField';
import { BoundDropdown, CheckYourScreenForPreview, WidgetSizeSelector } from '../shared';
import { AccountIcon, Datetime, ModalCustom, Button, InlineNotification, Link, Markdown } from '@/components';
import { Calendar as IconCalendar } from '@carbon/react/icons';
import { WidgetName } from '../WidgetName';
import css from '../shared.scss';

const $ = getID('manifest-form').get;

export interface WidgetManifestFormProps {
    manifest: null | pb.WidgetManifest;
    params: FormifiedParams;
    errors: null | ParamsFormErrors;
    onParamChange(key: string, value: FormifiedValue): void;

    timezones: pb.Timezone[];
    size?: pb.WidgetSize;
    sizeOptions?: Array<Exclude<pb.WidgetSize, pb.WidgetSize.UNSPECIFIED>>;
    onSizeChange?(size: pb.WidgetSize): void;

    accounts?: pb.Account[];
    credentialTypes?: pb.CredentialTypeLookup;
    credentialBindings?: Record<string, string>;
    onCredentialBindingChange?(slotKey: string, accountId: string): void;
}

export interface FormWidgetManifestProps extends WidgetManifestFormProps {
    isOpen: boolean;
    onSave(): void;
    onCancel(): void;
}

// The dropdown drops a null selection, so "no account" has to be a real item.
const UNBOUND = pb.create(pb.AccountSchema, { id: '', name: '' });

// A binding whose account is *gone* never arrives — `effective_bindings` drops it server-side.
// One whose account is the wrong type does: existence is all that filter checks,
// and a hand-edited config can mismatch a slot.
function isMisbound(slot: pb.CredentialSlotDefinition, accounts: pb.Account[], boundAccountId: string) {
    return !!boundAccountId && !accounts.some(a => a.id === boundAccountId && a.typeId === slot.typeId);
}

// Every option in one dropdown is filtered to the slot's type, so they share its artwork.
// `plainId` opts one out: the stand-in for a misbound slot needs an id to be selectable,
// and rendering it like a real account is what made it read as one.
function accountOption(icon: pb.Icon | undefined, plainId?: string) {
    return function AccountOption(account: pb.Account) {
        if (!account.id || account.id === plainId) return <span children={account.name} />;

        return (
            <div className={css.accountElement}>
                <div className={css.accountElementName}>
                    <AccountIcon icon={icon} size={18} />
                    <span children={account.name} />
                </div>
                <div className={css.accountElementDate}>
                    <IconCalendar size={16} />
                    <Datetime value={account.createdAt} format="%d.%m.%Y" />
                </div>
            </div>
        );
    };
}

interface CredentialSlotFieldProps {
    slot: pb.CredentialSlotDefinition;
    accounts: pb.Account[];
    credentialTypes: pb.CredentialTypeLookup;
    boundAccountId: string;
    error?: string;
    onChange(slotKey: string, accountId: string): void;
    /// False for a later slot of a type already covered,
    // so a widget declaring two of one type does not say
    // the same thing about it twice.
    firstOfType: boolean;
}

function CredentialSlotField(props: CredentialSlotFieldProps) {
    const { slot, accounts, credentialTypes, boundAccountId, error, onChange, firstOfType } = props;
    const { formatMessage } = useIntl();

    const none = { ...UNBOUND, name: formatMessage({ defaultMessage: '— None —' }) };
    const eligible = accounts.filter(a => a.typeId === slot.typeId);
    const icon = credentialTypes.get(slot.typeId)?.icon;
    const typeName = credentialTypes.get(slot.typeId)?.name ?? slot.typeId;

    const bound = boundAccountId ? eligible.find(a => a.id === boundAccountId) : undefined;
    const isMismatched = isMisbound(slot, accounts, boundAccountId);

    // The mismatched id needs an item of its own, or selecting `— None —`
    // would re-select the object already showing, onChange would never fire,
    // and the stale id would save — which the server then rejects.
    const mismatched = { ...UNBOUND, id: boundAccountId, name: formatMessage({ defaultMessage: '— Invalid —' }) };
    const items = isMismatched ? [none, mismatched, ...eligible] : [none, ...eligible];

    // On the control rather than in a notification below it:
    // the value shown *is* the error, and saving stays blocked until it is fixed.
    const mismatchError = isMismatched
        ? formatMessage({ defaultMessage: 'Takes a {type} account — pick another, or clear it.' }, { type: typeName })
        : undefined;

    return (
        <Fragment>
            <BoundDropdown<pb.Account>
                id={$(`credential-${slot.key}`)}
                labelText={
                    slot.required
                        ? formatMessage({ defaultMessage: '{label} (required)' }, { label: slot.label })
                        : slot.label
                }
                placeholderText={none.name}
                helperText={slot.description}
                items={items}
                value={bound ?? (isMismatched ? mismatched : none)}
                error={error ?? mismatchError}
                onChange={account => onChange(slot.key, account.id)}
                itemToString={a => a?.name ?? ''}
                itemToElement={accountOption(icon, isMismatched ? boundAccountId : undefined)}
            />

            {eligible.length === 0 && firstOfType ? (
                // Outranks the required-but-unbound warning: telling the operator
                // to bind one is no use while the only control is an empty dropdown.
                <InlineNotification
                    kind="info"
                    theme="inverse"
                    stretch
                    hideCloseButton
                    title={formatMessage({ defaultMessage: 'No matching account' })}
                    children={
                        <FormattedMessage
                            defaultMessage="No {type} account exists yet — <a>add one in Accounts</a>."
                            values={{
                                type: typeName,
                                a: ch => <Link href={URLS.pages.accounts} children={ch} />,
                            }}
                        />
                    }
                />
            ) : slot.required && !boundAccountId && eligible.length > 0 ? (
                <InlineNotification
                    kind="warning"
                    theme="inverse"
                    stretch
                    hideCloseButton
                    title={formatMessage({ defaultMessage: 'No account bound' })}
                    children={formatMessage(
                        { defaultMessage: 'Bind a {label} for this widget to work.' },
                        { label: slot.label },
                    )}
                />
            ) : null}
        </Fragment>
    );
}

export function WidgetManifestForm(props: WidgetManifestFormProps) {
    const {
        manifest,
        params,
        errors,
        onParamChange,
        timezones,
        size,
        sizeOptions,
        onSizeChange,
        accounts = [],
        credentialTypes = new Map(),
        credentialBindings = {},
        onCredentialBindingChange,
    } = props;
    const { formatMessage } = useIntl();

    if (!manifest) return null;

    const showSizeSelector = !!sizeOptions && sizeOptions.length > 0 && !!onSizeChange && size != null;
    const fieldErrors = (errors?.fields ?? {}) as Record<string, string[] | undefined>;
    const hasFieldErrors = Object.values(fieldErrors).some(errs => errs?.some(Boolean));
    const globalErrors = errors?.global?.filter(Boolean) ?? [];
    const showGlobalError = !hasFieldErrors && globalErrors.length > 0;

    return (
        <Form className={css.form}>
            {manifest.configHelp ? <Markdown source={manifest.configHelp} className={css.configHelp} /> : null}

            {showSizeSelector ? (
                <WidgetSizeSelector field={{ value: size, options: sizeOptions, onChange: onSizeChange }} />
            ) : null}

            {onCredentialBindingChange
                ? manifest.credentials.map((slot, i) => (
                      <CredentialSlotField
                          key={slot.key}
                          slot={slot}
                          accounts={accounts}
                          credentialTypes={credentialTypes}
                          boundAccountId={credentialBindings[slot.key] ?? ''}
                          error={errors?.credentials?.[slot.key]?.[0]}
                          onChange={onCredentialBindingChange}
                          firstOfType={manifest.credentials.findIndex(s => s.typeId === slot.typeId) === i}
                      />
                  ))
                : null}

            {manifest.params.map(def => (
                <ParamField
                    key={def.key}
                    id={$(`param-${def.key}`)}
                    definition={def}
                    value={params[def.key] ?? null}
                    error={fieldErrors[def.key]?.[0]}
                    onChange={onParamChange}
                    timezones={timezones}
                />
            ))}

            <CheckYourScreenForPreview />

            {showGlobalError ? (
                <InlineNotification
                    kind="error"
                    theme="inverse"
                    stretch
                    hideCloseButton
                    title={formatMessage({ defaultMessage: 'Error' })}
                    children={pb.renderFieldErrorsAsList(globalErrors)}
                />
            ) : null}
        </Form>
    );
}

export function FormWidgetManifest(props: FormWidgetManifestProps) {
    const { isOpen, onSave, onCancel, ...formProps } = props;
    const { manifest, errors, accounts = [], credentialBindings = {}, onCredentialBindingChange } = formProps;
    const { formatMessage } = useIntl();

    if (!manifest) return null;

    // An unbound slot still saves, deliberately — the operator may configure the rest
    // and go find the credentials.
    // A binding that fits no account is wrong rather than incomplete;
    // the server refuses it, so offering the click only earns a toast after the fact.
    const misbound =
        !!onCredentialBindingChange &&
        manifest.credentials.some(slot => isMisbound(slot, accounts, credentialBindings[slot.key] ?? ''));

    return (
        <ModalCustom
            id={$('dialog')}
            className={css.modal}
            selectorPrimaryFocus="form input,button"
            size="sm"
            open={isOpen}
            title={<WidgetName name={manifest.name} subname={manifest.subname} />}
            label={formatMessage({ defaultMessage: 'Configure Widget' })}
            onClose={onCancel}
            children={<WidgetManifestForm {...formProps} />}
            footer={
                <Button
                    id={$('done')}
                    kind="primary"
                    children={formatMessage({ defaultMessage: 'Done' })}
                    onClick={onSave}
                    disabled={hasFormErrors(errors ?? undefined) || misbound}
                />
            }
        />
    );
}
