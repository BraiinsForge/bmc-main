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

import { useIntl } from 'react-intl';
import * as pb from '@/proto';
import { Form, hasFormErrors } from '@/lib/form';
import { getID } from '../const';
import type { FormifiedParams, FormifiedValue, ParamsFormErrors } from '../../fn';

import { ParamField } from '@/components/ParamField';
import { CheckYourScreenForPreview, WidgetSizeSelector } from '../shared';
import { ModalCustom, Button, InlineNotification, Markdown } from '@/components';
import { WidgetName } from '../WidgetName';
import css from '../shared.scss';

const $ = getID('manifest-form').get;

export interface FormWidgetManifestProps {
    isOpen: boolean;
    onSave(): void;
    onCancel(): void;

    manifest: null | pb.WidgetManifest;
    params: FormifiedParams;
    errors: null | ParamsFormErrors;
    onParamChange(key: string, value: FormifiedValue): void;

    timezones: pb.Timezone[];
    size?: pb.WidgetSize;
    sizeOptions?: Array<Exclude<pb.WidgetSize, pb.WidgetSize.UNSPECIFIED>>;
    onSizeChange?(size: pb.WidgetSize): void;
}

export function FormWidgetManifest(props: FormWidgetManifestProps) {
    const {
        isOpen,
        onSave,
        onCancel,
        manifest,
        params,
        errors,
        onParamChange,
        timezones,
        size,
        sizeOptions,
        onSizeChange,
    } = props;
    const { formatMessage } = useIntl();

    if (!manifest) return null;

    const showSizeSelector = !!sizeOptions && sizeOptions.length > 0 && !!onSizeChange && size != null;
    const fieldErrors = (errors?.fields ?? {}) as Record<string, string[] | undefined>;
    const hasFieldErrors = Object.values(fieldErrors).some(errs => errs?.some(Boolean));
    const globalErrors = errors?.global?.filter(Boolean) ?? [];
    const showGlobalError = !hasFieldErrors && globalErrors.length > 0;

    const form = (
        <Form className={css.form}>
            {manifest.configHelp ? <Markdown source={manifest.configHelp} className={css.configHelp} /> : null}

            {showSizeSelector ? (
                <WidgetSizeSelector field={{ value: size, options: sizeOptions, onChange: onSizeChange }} />
            ) : null}

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
            children={form}
            footer={
                <Button
                    id={$('done')}
                    kind="primary"
                    children={formatMessage({ defaultMessage: 'Done' })}
                    onClick={onSave}
                    disabled={hasFormErrors(errors ?? undefined)}
                />
            }
        />
    );
}
